//! Raster classification denoising and shared-boundary polygonization.
mod geos;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use rayon::prelude::*;
use flate2::{
    Compression,
    write::{GzEncoder, ZlibEncoder},
};
use geos::{Geometry, Geos};
use libloading::Library;
use rusqlite::{Connection, params};
use rstar::{RTree,RTreeObject,AABB};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    ffi::{CString, c_char, c_void},
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};
type H = *mut c_void;
macro_rules! call { ($g:expr,$name:literal,$ty:ty $(,$arg:expr)*) => {{
    let f:libloading::Symbol<$ty>=$g.get(concat!($name,"\0").as_bytes())?; f($($arg),*)
}}}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    source: PathBuf,
    #[arg(long)]
    coast: PathBuf,
    #[arg(long)]
    baseline: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    gdal: PathBuf,
    #[arg(long)]
    geos: PathBuf,
    #[arg(long, default_value_t = 2048)]
    size: usize,
    #[arg(long, default_value_t = 9)]
    minimum_cells: usize,
    #[arg(long)]
    only_filter: Option<String>,
    #[arg(long)]
    progressive_lod: bool,
    #[arg(long,default_value_t=1)]
    water_detail: usize,
    #[arg(long)]
    filter_span_m: Option<f64>,
    #[arg(long)]
    extent_m: Option<f64>,
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=5))]
    only_level: Option<u8>,
    #[arg(long, value_delimiter = ',', conflicts_with = "only_level", value_parser = clap::value_parser!(u8).range(0..=5))]
    levels: Vec<u8>,
}
#[derive(Clone)]
struct WaterBounds{envelope:AABB<[f64;2]>,id:usize}
impl RTreeObject for WaterBounds{
    type Envelope=AABB<[f64;2]>;
    fn envelope(&self)->Self::Envelope{self.envelope}
}
fn water_underlay(src:&[u8],land:&[u8],w:usize,h:usize)->Vec<u8>{
    let mut out=src.to_vec();let mut queue=VecDeque::new();
    for i in 0..out.len(){
        if land[i]!=0&&src[i]==3{out[i]=255;}
        else if land[i]!=0&&src[i]<3{queue.push_back(i);}
    }
    while let Some(i)=queue.pop_front(){
        for j in neighbors(i,w,h){
            if out[j]==255{out[j]=out[i];queue.push_back(j);}
        }
    }
    for v in &mut out{if *v==255{*v=0;}}
    out
}
fn classify(v: u8) -> u8 {
    match v {
        10 | 95 => 1,
        60 => 2,
        80 => 3,
        _ => 0,
    }
}
// Synchronous categorical mode filter. Ties retain the original class.
// Water, ocean and domain perimeter are fixed; land can never acquire water.
fn majority(src: &[u8], land: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut dst = src.to_vec();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if land[i] == 0 || src[i] == 3 {
                continue;
            }
            let mut count = [0u8; 3];
            for yy in y - 1..=y + 1 {
                for xx in x - 1..=x + 1 {
                    let j = yy * w + xx;
                    if land[j] != 0 && src[j] < 3 {
                        count[src[j] as usize] += 1
                    }
                }
            }
            let best = *count.iter().max().unwrap();
            if count[src[i] as usize] < best {
                // Do not invent a majority in an ambiguous tie.
                let winners: Vec<_> = (0..3).filter(|&c| count[c] == best).collect();
                if winners.len() == 1 {
                    dst[i] = winners[0] as u8
                }
            }
        }
    }
    dst
}
// Integral counts avoid O(radius^2) work per pixel on coarse levels.
fn majority_radius(src: &[u8], land: &[u8], w: usize, h: usize, radius: usize) -> Vec<u8> {
    if radius == 0 {
        return src.to_vec();
    }
    let stride = w + 1;
    let mut sums = vec![vec![0u32; (w + 1) * (h + 1)]; 3];
    for c in 0..3 {
        for y in 0..h {
            let mut row = 0u32;
            for x in 0..w {
                let i = y * w + x;
                if land[i] != 0 && src[i] == c as u8 {
                    row += 1
                }
                sums[c][(y + 1) * stride + x + 1] = sums[c][y * stride + x + 1] + row;
            }
        }
    }
    let mut result = src.to_vec();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if land[i] == 0 || src[i] >= 3 {
                continue;
            }
            let x0 = x.saturating_sub(radius);
            let x1 = (x + radius + 1).min(w);
            let y0 = y.saturating_sub(radius);
            let y1 = (y + radius + 1).min(h);
            let mut counts = [0u32; 3];
            for c in 0..3 {
                let s = &sums[c];
                counts[c] = s[y1 * stride + x1] + s[y0 * stride + x0]
                    - s[y0 * stride + x1]
                    - s[y1 * stride + x0];
            }
            let best = *counts.iter().max().unwrap();
            if counts[src[i] as usize] < best {
                let winners: Vec<_> = (0..3).filter(|&c| counts[c] == best).collect();
                if winners.len() == 1 {
                    result[i] = winners[0] as u8
                }
            }
        }
    }
    result
}
// Exact symmetric box support for an even or odd number of source cells.
// Sliding column/row counts use O(width) temporary memory instead of a full
// multi-gigabyte integral image. Water and ocean never take part in voting.
fn majority_window(src:&[u8],land:&[u8],w:usize,h:usize,width_cells:usize)->Vec<u8>{
    if width_cells<=1{return src.to_vec()}
    let radius=width_cells/2;let half=width_cells%2==0;
    let mut result=src.to_vec();
    const BAND_ROWS: usize = 512;
    result.par_chunks_mut(w * BAND_ROWS).enumerate().for_each(|(band, output)| {
    let first = band * BAND_ROWS;
    let end = first + output.len() / w;
    let mut columns=vec![[0u32;3];w];let mut vertical=columns.clone();
    for y in first.saturating_sub(radius)..=(first+radius).min(h-1){for x in 0..w{let i=y*w+x;if land[i]!=0&&src[i]<3{columns[x][src[i]as usize]+=1;}}}
    for y in first..end{
        if y>first{
            if y>radius{let row=y-radius-1;for x in 0..w{let i=row*w+x;if land[i]!=0&&src[i]<3{columns[x][src[i]as usize]-=1;}}}
            if y+radius<h{let row=y+radius;for x in 0..w{let i=row*w+x;if land[i]!=0&&src[i]<3{columns[x][src[i]as usize]+=1;}}}
        }
        for x in 0..w{
            vertical[x]=columns[x];
            if half{
                for c in 0..3{vertical[x][c]*=2;}
                for row in [y.checked_sub(radius),if y+radius<h{Some(y+radius)}else{None}].into_iter().flatten(){
                    let i=row*w+x;if land[i]!=0&&src[i]<3{vertical[x][src[i]as usize]-=1;}
                }
            }
        }
        let mut sum=[0u32;3];
        for v in vertical.iter().take((radius+1).min(w)){for c in 0..3{sum[c]+=v[c];}}
        for x in 0..w{
            if x>0{
                if x>radius{for c in 0..3{sum[c]-=vertical[x-radius-1][c];}}
                if x+radius<w{for c in 0..3{sum[c]+=vertical[x+radius][c];}}
            }
            let i=y*w+x;if y==0||x==0||y+1==h||x+1==w||land[i]==0||src[i]>=3{continue}
            let mut counts=sum;
            if half{
                for c in 0..3{counts[c]*=2;}
                for col in [x.checked_sub(radius),if x+radius<w{Some(x+radius)}else{None}].into_iter().flatten(){
                    for c in 0..3{counts[c]-=vertical[col][c];}
                }
            }
            let best=*counts.iter().max().unwrap();
            if counts[src[i]as usize]<best{
                let mut winner=None;let mut tie=false;
                for c in 0..3{if counts[c]==best{if winner.is_some(){tie=true;}winner=Some(c);}}
                if !tie{output[i-first*w]=winner.unwrap()as u8;}
            }
        }
    }
    });
    result
}
// Four-connected components; merge whole small regions, never isolated pixels
// from one region into multiple recipients. Larger-neighbor parent graph is acyclic.
fn small_regions(src: &[u8], land: &[u8], w: usize, h: usize, minimum: usize) -> Vec<u8> {
    if minimum <= 1 {
        return src.to_vec();
    }
    let mut ids = vec![u32::MAX; src.len()];
    let mut q = VecDeque::new();
    let mut sizes = Vec::new();
    let mut codes = Vec::new();
    let mut border = Vec::new();
    for start in 0..src.len() {
        if ids[start] != u32::MAX || land[start] == 0 {
            continue;
        }
        let id = u32::try_from(sizes.len()).expect("Too many connected regions");
        let code = src[start];
        q.push_back(start);
        ids[start] = id;
        let mut size = 0;
        let mut edge = false;
        while let Some(i) = q.pop_front() {
            size += 1;
            let x = i % w;
            let y = i / w;
            edge |= x == 0 || y == 0 || x + 1 == w || y + 1 == h;
            for j in neighbors(i, w, h) {
                if land[j] == 0 {
                    edge = true
                } else if ids[j] == u32::MAX && src[j] == code {
                    ids[j] = id;
                    q.push_back(j)
                }
            }
        }
        sizes.push(size);
        codes.push(code);
        border.push(edge);
    }
    let eligible: Vec<_> = (0..sizes.len())
        .map(|a| codes[a] < 3 && sizes[a] < minimum && !border[a])
        .collect();
    let mut adjacency = vec![HashMap::<usize, usize>::new(); sizes.len()];
    for i in 0..src.len() {
        let a = ids[i];
        if a == u32::MAX {
            continue;
        }
        for j in [
            if i % w + 1 < w { Some(i + 1) } else { None },
            if i / w + 1 < h { Some(i + w) } else { None },
        ]
        .into_iter()
        .flatten()
        {
            let b = ids[j];
            if b != u32::MAX && a != b {
                let (a, b) = (a as usize, b as usize);
                if eligible[a] && codes[b] < 3 {
                    *adjacency[a].entry(b).or_default() += 1;
                }
                if eligible[b] && codes[a] < 3 {
                    *adjacency[b].entry(a).or_default() += 1;
                }
            }
        }
    }
    let mut parent: Vec<_> = (0..sizes.len()).collect();
    for a in 0..sizes.len() {
        if !eligible[a] {
            continue;
        }
        let best = adjacency[a]
            .iter()
            .filter(|&(&b, _)| {
                codes[b] < 3 && (sizes[b] > sizes[a] || (sizes[b] == sizes[a] && b < a))
            })
            .max_by_key(|&(&b, &length)| (length, sizes[b], std::cmp::Reverse(b)));
        if let Some((&b, _)) = best {
            parent[a] = b;
        }
    }
    let mut result = src.to_vec();
    for i in 0..src.len() {
        if ids[i] == u32::MAX {
            continue;
        }
        let mut a = ids[i] as usize;
        while parent[a] != a {
            parent[a] = parent[parent[a]];
            a = parent[a]
        }
        result[i] = codes[a];
    }
    result
}
const WATER_HOLE_M2: [f64; 6] = [0., 0., 500., 4000., 40000., 160000.];

const WATER_BANK_M: [f64; 6] = [0., 16., 40., 100., 250., 500.];
const WATER_AREA_M2: [f64; 6] = [0., 500., 2500., 20000., 100000., 400000.];
const WATER_EXTENT_M: [f64; 6] = [0., 50., 100., 300., 650., 1300.];

fn fill_small_water_holes(
    api: &Geos,
    faces: &[Geometry<'_>],
    codes: &mut [usize],
    max_area: f64,
) -> Result<()> {
    let mut holes = Vec::new();
    let mut bounds = Vec::new();
    for (face, &code) in faces.iter().zip(codes.iter()) {
        if code != 3 {
            continue;
        }
        for mut ring in face.rings()?.into_iter().skip(1) {
            if ring.first() == ring.last() {
                ring.pop();
            }
            let hole = api.polygon(&ring)?;
            if hole.area()? >= max_area {
                continue;
            }
            let mut lo = [f64::INFINITY; 2];
            let mut hi = [f64::NEG_INFINITY; 2];
            for xy in ring {
                for axis in 0..2 {
                    lo[axis] = lo[axis].min(xy[axis]);
                    hi[axis] = hi[axis].max(xy[axis]);
                }
            }
            bounds.push(WaterBounds { envelope: AABB::from_corners(lo, hi), id: holes.len() });
            holes.push(hole);
        }
    }
    let index = RTree::bulk_load(bounds);
    for (face, code) in faces.iter().zip(codes.iter_mut()) {
        if *code >= 3 {
            continue;
        }
        let [x, y] = face.interior_xy()?;
        for hit in index.locate_in_envelope_intersecting(&AABB::from_point([x, y])) {
            if holes[hit.id].covers_xy(x, y)? {
                // Faces have already been noded against the water boundary.
                *code = 3;
                break;
            }
        }
    }
    Ok(())
}

fn prune_inland_water(values: &mut [u8], land: &[u8], n: usize, res: f64, level: usize) -> usize {
    let mut seen = vec![false; values.len()];
    let mut cells = Vec::new();
    let mut removed = 0;
    for start in 0..values.len() {
        if values[start] != 3 || land[start] == 0 || seen[start] {
            continue;
        }
        cells.clear();
        cells.push(start);
        seen[start] = true;
        let (mut lo, mut hi) = ([n; 2], [0; 2]);
        let mut edge = false;
        let mut dry = [0usize; 3];
        let mut cursor = 0;
        while cursor < cells.len() {
            let i = cells[cursor];
            cursor += 1;
            let (x, y) = (i % n, i / n);
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
            edge |= x == 0 || y == 0 || x + 1 == n || y + 1 == n;
            // Keep diagonal chains together, but count only shared sides when
            // choosing which adjacent land class will replace a small patch.
            for ny in y.saturating_sub(1)..=(y + 1).min(n - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(n - 1) {
                    let j = ny * n + nx;
                    let side = x.abs_diff(nx) + y.abs_diff(ny) == 1;
                    if land[j] == 0 {
                        edge |= side;
                    } else if values[j] == 3 && !seen[j] {
                        seen[j] = true;
                        cells.push(j);
                    } else if side && values[j] < 3 {
                        dry[values[j] as usize] += 1;
                    }
                }
            }
        }
        let area = cells.len() as f64 * res * res;
        let extent = ((hi[0] - lo[0] + 1) as f64).hypot((hi[1] - lo[1] + 1) as f64) * res;
        if edge || area >= WATER_AREA_M2[level] || extent >= WATER_EXTENT_M[level] {
            continue;
        }
        let code = (0..dry.len()).max_by_key(|&c| (dry[c], std::cmp::Reverse(c))).unwrap();
        if dry[code] > 0 {
            for &i in &cells {
                values[i] = code as u8;
            }
            removed += 1;
        }
    }
    removed
}

fn neighbors(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let x = i % w;
    let y = i / w;
    [
        if x > 0 { Some(i - 1) } else { None },
        if x + 1 < w { Some(i + 1) } else { None },
        if y > 0 { Some(i - w) } else { None },
        if y + 1 < h { Some(i + w) } else { None },
    ]
    .into_iter()
    .flatten()
}
unsafe fn raster(
    g: &Library,
    w: usize,
    h: usize,
    geo: &mut [f64; 6],
    data: &mut [u8],
) -> Result<H> {
    unsafe {
        let driver = call!(
            g,
            "GDALGetDriverByName",
            unsafe extern "C" fn(*const c_char) -> H,
            c"MEM".as_ptr()
        );
        let ds = call!(
            g,
            "GDALCreate",
            unsafe extern "C" fn(H, *const c_char, i32, i32, i32, i32, *mut *mut c_char) -> H,
            driver,
            c"".as_ptr(),
            w as i32,
            h as i32,
            1,
            1,
            std::ptr::null_mut()
        );
        ensure!(!ds.is_null(), "GDAL MEM create failed");
        ensure!(
            call!(
                g,
                "GDALSetGeoTransform",
                unsafe extern "C" fn(H, *mut f64) -> i32,
                ds,
                geo.as_mut_ptr()
            ) == 0,
            "geotransform"
        );
        io(g, ds, 1, w, h, data)?;
        Ok(ds)
    }
}
unsafe fn io(g: &Library, ds: H, write: i32, w: usize, h: usize, data: &mut [u8]) -> Result<()> {
    unsafe {
        let band = call!(
            g,
            "GDALGetRasterBand",
            unsafe extern "C" fn(H, i32) -> H,
            ds,
            1
        );
        ensure!(
            call!(
                g,
                "GDALRasterIO",
                unsafe extern "C" fn(
                    H,
                    i32,
                    i32,
                    i32,
                    i32,
                    i32,
                    *mut c_void,
                    i32,
                    i32,
                    i32,
                    i32,
                    i32,
                ) -> i32,
                band,
                write,
                0,
                0,
                w as i32,
                h as i32,
                data.as_mut_ptr().cast(),
                w as i32,
                h as i32,
                1,
                0,
                0
            ) == 0,
            "RasterIO"
        );
        Ok(())
    }
}
unsafe fn close(g: &Library, ds: H) -> Result<()> {
    unsafe {
        call!(g, "GDALClose", unsafe extern "C" fn(H), ds);
        Ok(())
    }
}
unsafe fn sample(g: &Library, path: &Path, b: [f64; 4], n: usize) -> Result<Vec<u8>> {
    unsafe {
        let name = CString::new(path.to_string_lossy().as_bytes())?;
        let ds = call!(
            g,
            "GDALOpen",
            unsafe extern "C" fn(*const c_char, i32) -> H,
            name.as_ptr(),
            0
        );
        ensure!(!ds.is_null(), "Open source");
        let mut tr = [0.; 6];
        ensure!(
            call!(
                g,
                "GDALGetGeoTransform",
                unsafe extern "C" fn(H, *mut f64) -> i32,
                ds,
                tr.as_mut_ptr()
            ) == 0,
            "source geotransform"
        );
        ensure!(
            tr[2] == 0. && tr[4] == 0. && tr[1] > 0. && tr[5] < 0.,
            "Expected north-up EPSG:4326 source"
        );
        let w = call!(g, "GDALGetRasterXSize", unsafe extern "C" fn(H) -> i32, ds);
        let h = call!(g, "GDALGetRasterYSize", unsafe extern "C" fn(H) -> i32, ds);
        let lon = |x: f64| x / 6378137. * 180. / std::f64::consts::PI;
        let lat = |y: f64| (y / 6378137.).sinh().atan() * 180. / std::f64::consts::PI;
        let x0 = (((lon(b[0]) - tr[0]) / tr[1]).floor() as i32).clamp(0, w - 1);
        let x1 = (((lon(b[2]) - tr[0]) / tr[1]).ceil() as i32).clamp(1, w);
        let y0 = (((lat(b[3]) - tr[3]) / tr[5]).floor() as i32).clamp(0, h - 1);
        let y1 = (((lat(b[1]) - tr[3]) / tr[5]).ceil() as i32).clamp(1, h);
        ensure!(x1 > x0 && y1 > y0, "source window");
        let sw = (x1 - x0) as usize;
        let sh = (y1 - y0) as usize;
        let mut source = vec![0u8; sw * sh];
        let band = call!(
            g,
            "GDALGetRasterBand",
            unsafe extern "C" fn(H, i32) -> H,
            ds,
            1
        );
        ensure!(
            call!(
                g,
                "GDALRasterIO",
                unsafe extern "C" fn(
                    H,
                    i32,
                    i32,
                    i32,
                    i32,
                    i32,
                    *mut c_void,
                    i32,
                    i32,
                    i32,
                    i32,
                    i32,
                ) -> i32,
                band,
                0,
                x0,
                y0,
                sw as i32,
                sh as i32,
                source.as_mut_ptr().cast(),
                sw as i32,
                sh as i32,
                1,
                0,
                0
            ) == 0,
            "read source"
        );
        close(g, ds)?;
        let columns:Vec<_>=(0..n).map(|x|{
            let mx=b[0]+(x as f64+0.5)*(b[2]-b[0])/n as f64;
            ((lon(mx)-tr[0])/tr[1]).floor()as i32
        }).collect();
        let rows:Vec<_>=(0..n).map(|y|{
            let my=b[3]-(y as f64+0.5)*(b[3]-b[1])/n as f64;
            ((lat(my)-tr[3])/tr[5]).floor()as i32
        }).collect();
        ensure!(columns.iter().all(|&x|x>=x0&&x<x1)&&rows.iter().all(|&y|y>=y0&&y<y1),"source extent missing");
        let mut out=vec![0;n*n];
        for(y,&yy)in rows.iter().enumerate(){
            for(x,&xx)in columns.iter().enumerate(){
                out[y*n+x]=classify(source[(yy-y0)as usize*sw+(xx-x0)as usize]);
            }
        }
        Ok(out)
    }
}
unsafe fn coast_mask(g: &Library, wkb: &[u8], bounds: [f64; 4], n: usize) -> Result<Vec<u8>> {
    unsafe {
        let mut a = vec![0; n * n];
        let mut tr = [
            bounds[0],
            (bounds[2] - bounds[0]) / n as f64,
            0.,
            bounds[3],
            0.,
            -(bounds[2] - bounds[0]) / n as f64,
        ];
        let ds = raster(g, n, n, &mut tr, &mut a)?;
        let mut geom = std::ptr::null_mut();
        ensure!(
            call!(
                g,
                "OGR_G_CreateFromWkb",
                unsafe extern "C" fn(*const u8, H, *mut H, i32) -> i32,
                wkb.as_ptr(),
                std::ptr::null_mut(),
                &mut geom,
                wkb.len() as i32
            ) == 0,
            "coast WKB"
        );
        let band = 1;
        let burn = 1.;
        ensure!(
            call!(
                g,
                "GDALRasterizeGeometries",
                unsafe extern "C" fn(
                    H,
                    i32,
                    *const i32,
                    i32,
                    *const H,
                    H,
                    H,
                    *const f64,
                    H,
                    H,
                    H,
                ) -> i32,
                ds,
                1,
                &band,
                1,
                &geom,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &burn,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ) == 0,
            "coast rasterize"
        );
        call!(g, "OGR_G_DestroyGeometry", unsafe extern "C" fn(H), geom);
        io(g, ds, 0, n, n, &mut a)?;
        close(g, ds)?;
        Ok(a)
    }
}
unsafe fn polygonize<'a>(g:&Library,api:&'a Geos,values:&mut[u8],w:usize,h:usize,tr:&mut[f64;6])->Result<Vec<(u8,Geometry<'a>)>>{
    unsafe{polygonize_masked(g,api,values,w,h,tr,false)}
}
unsafe fn polygonize_masked<'a>(
    g: &Library,
    api: &'a Geos,
    values: &mut [u8],
    w: usize,
    h: usize,
    tr: &mut [f64; 6],
    masked:bool,
) -> Result<Vec<(u8, Geometry<'a>)>> {
    unsafe {
        let ds = raster(g, w, h, tr, values)?;
        let driver = call!(
            g,
            "GDALGetDriverByName",
            unsafe extern "C" fn(*const c_char) -> H,
            c"Memory".as_ptr()
        );
        let vector = call!(
            g,
            "GDALCreate",
            unsafe extern "C" fn(H, *const c_char, i32, i32, i32, i32, *mut *mut c_char) -> H,
            driver,
            c"".as_ptr(),
            0,
            0,
            0,
            0,
            std::ptr::null_mut()
        );
        ensure!(!vector.is_null(), "Memory vector");
        let layer = call!(
            g,
            "GDALDatasetCreateLayer",
            unsafe extern "C" fn(H, *const c_char, H, i32, H) -> H,
            vector,
            c"regions".as_ptr(),
            std::ptr::null_mut(),
            3,
            std::ptr::null_mut()
        );
        ensure!(!layer.is_null(), "create vector layer");
        let field = call!(
            g,
            "OGR_Fld_Create",
            unsafe extern "C" fn(*const c_char, i32) -> H,
            c"class".as_ptr(),
            0
        );
        ensure!(
            call!(
                g,
                "OGR_L_CreateField",
                unsafe extern "C" fn(H, H, i32) -> i32,
                layer,
                field,
                1
            ) == 0,
            "field"
        );
        call!(g, "OGR_Fld_Destroy", unsafe extern "C" fn(H), field);
        let band = call!(
            g,
            "GDALGetRasterBand",
            unsafe extern "C" fn(H, i32) -> H,
            ds,
            1
        );
        ensure!(
            call!(
                g,
                "GDALPolygonize",
                unsafe extern "C" fn(H, H, H, i32, H, H, H) -> i32,
                band,
                if masked{band}else{std::ptr::null_mut()},
                layer,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ) == 0,
            "polygonize"
        );
        call!(g, "OGR_L_ResetReading", unsafe extern "C" fn(H), layer);
        let mut out = Vec::new();
        loop {
            let f = call!(
                g,
                "OGR_L_GetNextFeature",
                unsafe extern "C" fn(H) -> H,
                layer
            );
            if f.is_null() {
                break;
            }
            let code = call!(
                g,
                "OGR_F_GetFieldAsInteger",
                unsafe extern "C" fn(H, i32) -> i32,
                f,
                0
            ) as u8;
            let geom = call!(g, "OGR_F_GetGeometryRef", unsafe extern "C" fn(H) -> H, f);
            let size = call!(g, "OGR_G_WkbSize", unsafe extern "C" fn(H) -> i32, geom);
            ensure!(size > 0, "WKB size");
            let mut bytes = vec![0; size as usize];
            ensure!(
                call!(
                    g,
                    "OGR_G_ExportToWkb",
                    unsafe extern "C" fn(H, i32, *mut u8) -> i32,
                    geom,
                    1,
                    bytes.as_mut_ptr()
                ) == 0,
                "export WKB"
            );
            out.push((code, api.read(&bytes)?));
            call!(g, "OGR_F_Destroy", unsafe extern "C" fn(H), f);
        }
        close(g, vector)?;
        close(g, ds)?;
        Ok(out)
    }
}
fn detailed_water<'a>(g:&Library,api:&'a Geos,args:&Args,bounds:[f64;4],worldland:&Geometry<'a>)->Result<Geometry<'a>>{
    let n=args.size*args.water_detail;let step=n/8;let side=bounds[2]-bounds[0];let res=side/n as f64;let tile_side=side/8.;
    println!("Detailed water: {n} x {n}, {res} projected m");
    let mut labels=unsafe{sample(g,&args.source,bounds,n)}?;
    for v in &mut labels{*v=if *v==3{1}else{0};}
    let mut pieces=Vec::new();
    for by in 0..8{for bx in 0..8{
        let left=bounds[0]+bx as f64*tile_side;let top=bounds[3]-by as f64*tile_side;
        let tile=api.rectangle(left,top-tile_side,left+tile_side,top)?;
        let land=worldland.intersection(&tile)?;
        let mut block:Vec<_>=(by*step..(by+1)*step).flat_map(|y|labels[y*n+bx*step..y*n+(bx+1)*step].iter().copied()).collect();
        let mut tr=[left,res,0.,top,0.,-res];
        for(_,p)in unsafe{polygonize_masked(g,api,&mut block,step,step,&mut tr,true)}?{
            if land.covers(&p)?{pieces.push(p)}
            else if land.intersects(&p)?{pieces.extend(p.intersection(&land)?.polygons()?);}
        }
    }println!("Detailed water strip {}/8",by+1);}
    let result=api.collection(&pieces)?.union()?;
    ensure!(result.is_valid()?,"Invalid detailed water");
    println!("Detailed inland water: {} polygons",result.polygons()?.len());
    Ok(result)
}
fn compress(raw: &[u8], gzip: bool) -> Result<Vec<u8>> {
    if gzip {
        let mut w = GzEncoder::new(Vec::new(), Compression::best());
        w.write_all(raw)?;
        Ok(w.finish()?)
    } else {
        let mut w = ZlibEncoder::new(Vec::new(), Compression::best());
        w.write_all(raw)?;
        Ok(w.finish()?)
    }
}
fn png_labels(path:&Path,src:&[u8],mask:&[u8],n:usize)->Result<()>{
    let display=n.min(2048);
    let mut writer=png::Encoder::new(fs::File::create(path)?,display as u32,display as u32);
    writer.set_color(png::ColorType::Indexed);writer.set_depth(png::BitDepth::Eight);
    writer.set_palette(vec![184,190,158,82,112,70,171,160,137,76,132,166,134,180,203]);
    let mut values=Vec::with_capacity(display*display);
    for y in 0..display{for x in 0..display{
        let row=((y as f64+0.5)*n as f64/display as f64)as usize;
        let col=((x as f64+0.5)*n as f64/display as f64)as usize;let i=row*n+col;
        values.push(if mask[i]==0{4}else{src[i]});
    }}
    writer.write_header()?.write_image_data(&values)?;Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.size >= 256 && args.size % 256 == 0,
        "size must be a multiple of 256"
    );
    ensure!(
        !args.output.join("manifest.json").exists(),
        "Choose a new output directory"
    );
    fs::create_dir_all(&args.output)?;
    let mut base: Value = serde_json::from_slice(&fs::read(args.baseline.join("manifest.json"))?)?;
    let bounds: Vec<f64> = serde_json::from_value(base["bounds"].clone())?;
    let mut bounds: [f64; 4] = bounds.try_into().unwrap();
    if let Some(side) = args.extent_m {
        ensure!(side.is_finite() && side > 0., "Invalid map extent");
        let cx = (bounds[0] + bounds[2]) * 0.5;
        let cy = (bounds[1] + bounds[3]) * 0.5;
        let half = side * 0.5;
        bounds = [cx - half, cy - half, cx + half, cy + half];
        base["bounds"] = json!(bounds);
        base["side"] = json!(side);
        base["title"] = json!(format!("富士山 {} km・地表ベクトルLOD", side / 1000.));
        base["sourceFile"] = json!(args.source.file_name().and_then(|name| name.to_str()));
        if let Some(metadata) = base.as_object_mut() {
            metadata.remove("relief");
        }
    }
    let side = bounds[2] - bounds[0];
    let started = Instant::now();
    // GDAL registers process-wide drivers and TLS. Keep the DLL loaded until
    // process exit instead of running its unload hooks through FreeLibrary.
    let g = std::mem::ManuallyDrop::new(
        unsafe { Library::new(&args.gdal) }.context("Load GDAL (add DLL directory to PATH)")?,
    );
    unsafe {
        call!(&g, "GDALAllRegister", unsafe extern "C" fn());
    }
    let api = Geos::open(&args.geos)?;
    let coastbytes = fs::read(&args.coast)?;
    let worldland = api
        .read(&coastbytes)?
        .intersection(&api.rectangle(bounds[0], bounds[1], bounds[2], bounds[3])?)?;
    let n = args.size;
    let res = (bounds[2] - bounds[0]) / n as f64;
    let sampled = unsafe { sample(&g, &args.source, bounds, n) }?;
    let mask = unsafe { coast_mask(&g, &coastbytes, bounds, n) }?;
    ensure!([1,2,4,8].contains(&args.water_detail),"water-detail must be 1/2/4/8");
    let raw=if args.water_detail>1{water_underlay(&sampled,&mask,n,n)}else{sampled.clone()};
    let high_water=if args.water_detail>1{Some(detailed_water(&g,&api,&args,bounds,&worldland)?)}else{None};
    println!(
        "Source sampled {n} x {n}; {res} projected m, {:.2}s",
        started.elapsed().as_secs_f64()
    );
    let full_resolution=args.filter_span_m.is_some();
    let cell_scale=if full_resolution{(62.5/res).powi(2)}else{1.};
    let base_minimum=(args.minimum_cells as f64*cell_scale).ceil()as usize;
    let width_cells=if let Some(span)=args.filter_span_m {
        ensure!(span.is_finite()&&span>0.,"Invalid filter span");
        (span/res).round().max(1.)as usize
    }else{3};
    let selected:Vec<_>=if let Some(ref f)=args.only_filter{f.split(',').collect()}else{vec!["none","majority","majority-clean","regions"]};
    ensure!(selected.iter().all(|f|["none","majority","majority-clean","regions"].contains(f)),"Unknown filter");
    let mut results=Vec::new();
    for mode in selected {
        let label=match mode{"none"=>"なし","majority"=>"多数決","majority-clean"=>"多数決＋孤立領域除去","regions"=>"小領域除去",_=>unreachable!()};
        println!("{mode}: source-resolution preprocessing, window {width_cells} cells, minimum {base_minimum} cells");
        let values=match mode {
            "none"=>raw.clone(),
            "majority"=>majority_window(&raw,&mask,n,n,width_cells),
            "majority-clean"=>{
                let voted=majority_window(&raw,&mask,n,n,width_cells);
                small_regions(&voted,&mask,n,n,base_minimum)
            },
            "regions"=>small_regions(&raw,&mask,n,n,base_minimum),
            _=>unreachable!()
        };
        let now = Instant::now();
        let out = args.output.join(mode);
        fs::create_dir_all(&out)?;
        let changed = values.iter().zip(&raw).filter(|(a, b)| a != b).count();
        ensure!(
            (0..raw.len()).all(|i| (raw[i] != 3 && values[i] != 3) || raw[i] == values[i]),
            "Water changed"
        );
        ensure!(
            (0..raw.len()).all(|i| mask[i] != 0 || raw[i] == values[i]),
            "Ocean changed"
        );
        png_labels(&out.join("reference.png"), &values, &mask, n)?;
        if args.extent_m.is_none() && args.baseline.join("relief.png").exists() {
            fs::copy(args.baseline.join("relief.png"), out.join("relief.png"))?;
        }

        let mut levels: Vec<Vec<Vec<Geometry<'_>>>> = (0..6)
            .map(|_| (0..5).map(|_| Vec::new()).collect())
            .collect();
        let tolerances: [f64; 6] = [0., 25., 75., 200., 500., 1000.];
        let radii = [0usize, 1, 2, 4, 8, 16];
        let thresholds=[9usize,16,64,256,1024,4096].map(|v|{
            ((v.max(args.minimum_cells)as f64)*cell_scale).ceil()as usize
        });
        let selected_level = |level| {
            if args.levels.is_empty() {
                args.only_level.map_or(level < 5, |selected| level == usize::from(selected))
            } else {
                args.levels.contains(&(level as u8))
            }
        };
        let mut level_changes = [0; 6];
        for (l, &t) in tolerances.iter().enumerate().filter(|(l, _)| selected_level(*l)) {
            let mut level_values = if args.progressive_lod && l > 0 {
                let filtered=if full_resolution{
                    let span=62.5*(2*radii[l]+1)as f64;
                    majority_window(&values,&mask,n,n,(span/res).round().max(1.)as usize)
                }else{majority_radius(&values,&mask,n,n,radii[l])};
                small_regions(&filtered, &mask, n, n, thresholds[l])
            } else {
                values.clone()
            };
            ensure!((0..raw.len()).all(|i|level_values[i]!=3&&values[i]!=3||level_values[i]==values[i]),"Water changed in LOD");
            ensure!(
                (0..raw.len()).all(|i| mask[i] != 0 || level_values[i] == values[i]),
                "Sea changed in LOD"
            );
            if full_resolution && args.progressive_lod && high_water.is_none() && l > 0 {
                let removed = prune_inland_water(&mut level_values, &mask, n, res, l);
                eprintln!("{mode} L{l}: omitted {removed} small inland water components");
            }
            level_changes[l] =
                level_values
                    .iter()
                    .zip(&values)
                    .filter(|(a, b)| a != b)
                    .count();
            let mut reference=level_values.clone();
            if args.water_detail>1 {for i in 0..reference.len(){if sampled[i]==3{reference[i]=3;}}}
            png_labels(
                &out.join(format!("reference-{l}.png")),
                &reference,
                &mask,
                n,
            )?;
            let values = &level_values;
            let water_tolerances=[0.,4.,8.,16.,32.,64.];
            let water_parts=if let Some(w)=&high_water{
                let simplified=w.topology_simplify(water_tolerances[l])?;
                ensure!(simplified.is_valid()?,"Invalid simplified water");
                simplified.polygons()?
            }else{Vec::new()};
            let mut water_boxes=Vec::new();
            for(id,p)in water_parts.iter().enumerate(){
                let coords=p.rings()?;let mut lo=[f64::INFINITY;2];let mut hi=[f64::NEG_INFINITY;2];
                for xy in coords.iter().flatten(){for axis in 0..2{lo[axis]=lo[axis].min(xy[axis]);hi[axis]=hi[axis].max(xy[axis]);}}
                water_boxes.push(WaterBounds{envelope:AABB::from_corners(lo,hi),id});
            }
            let water_tree=RTree::bulk_load(water_boxes);
            let batch=n/8;
            for by in (0..n).step_by(batch) {
                for bx in (0..n).step_by(batch) {
                    let mut block: Vec<_> = (by..by + batch)
                        .flat_map(|y| values[y * n + bx..y * n + bx + batch].iter().copied())
                        .collect();
                    let left = bounds[0] + bx as f64 * res;
                    let top = bounds[3] - by as f64 * res;
                    let tile = api.rectangle(left, top - batch as f64 * res, left + batch as f64 * res, top)?;
                    let land = worldland.intersection(&tile)?;
                    let water=if high_water.is_some(){
                        let envelope=AABB::from_corners([left,top-batch as f64*res],[left+batch as f64*res,top]);
                        let mut parts=Vec::new();
                        for hit in water_tree.locate_in_envelope_intersecting(&envelope){
                            parts.extend(water_parts[hit.id].intersection(&tile)?.polygons()?);
                        }
                        Some(api.collection(&parts)?.union()?.intersection(&land)?)
                    }else{None};
                    let dry_land=if let Some(w)=&water{Some(land.difference(w)?)}else{None};
                    let class_land=dry_land.as_ref().unwrap_or(&land);
                    let mut tr = [left, res, 0., top, 0., -res];
                    let mut faces = Vec::new();
                    let mut codes = Vec::new();
                    for (code, poly) in
                        unsafe { polygonize(&g, &api, &mut block, batch, batch, &mut tr) }?
                    {
                        let clipped = if class_land.covers(&poly)? {
                            poly
                        } else if class_land.intersects(&poly)? {
                            poly.intersection(class_land)?
                        } else {
                            continue;
                        };
                        for p in clipped.polygons()? {
                            faces.push(p);
                            codes.push(code as usize)
                        }
                    }
                    if let Some(w)=&water{for p in w.polygons()?{faces.push(p);codes.push(3);}}
                    for p in tile.difference(&land)?.polygons()? {
                        faces.push(p);
                        codes.push(4)
                    }
                    // Raster polygons may omit collinear T-junction nodes. Node the
                    // entire shared graph before coverage simplification.
                    let lines: Vec<_> =
                        faces.iter().map(|p| p.boundary()).collect::<Result<_>>()?;
                    let network = api.collection(&lines)?.union()?;
                    let faces = network.polygonize()?.polygons()?;
                    let mut codes = Vec::new();
                    for p in &faces {
                        let [x, y] = p.interior_xy()?;
                        let code = if !land.covers_xy(x, y)? {
                            4
                        } else if water.as_ref().map(|w|w.covers_xy(x,y)).transpose()?.unwrap_or(false){
                            3
                        } else {
                            let col = (((x - bounds[0]) / res).floor() as usize).min(n - 1);
                            let row = (((bounds[3] - y) / res).floor() as usize).min(n - 1);
                            values[row * n + col] as usize
                        };
                        codes.push(code);
                    }

                    if full_resolution && args.progressive_lod && high_water.is_none() && WATER_HOLE_M2[l] > 0. {
                        fill_small_water_holes(&api, &faces, &mut codes, WATER_HOLE_M2[l])?;
                    }

                    let all = api
                        .collection(&faces)?
                        .coverage_simplify(if args.progressive_lod { 0. } else { t.min(16.) })?;
                    let mut simplified = all.polygons()?;
                    ensure!(
                        simplified.len() == codes.len(),
                        "coverage changed polygon order"
                    );
                    if args.filter_span_m.is_some() && args.progressive_lod && high_water.is_none() && l > 0 {
                        // Simplify each bank together with its adjacent dry faces.
                        // The outer boundary of this coverage includes the coast and tile edges.
                        let indices: Vec<_> = (0..codes.len()).filter(|&i| codes[i] < 4).collect();
                        if !indices.is_empty() {
                            let land_faces: Vec<_> = indices
                                .iter()
                                .map(|&i| api.read(&simplified[i].write()?))
                                .collect::<Result<_>>()?;
                            let land_faces = api.collection(&land_faces)?
                                .coverage_simplify(WATER_BANK_M[l])?.polygons()?;
                            ensure!(land_faces.len() == indices.len(), "land polygon count");
                            for (i, p) in indices.into_iter().zip(land_faces) {
                                simplified[i] = p;
                            }
                        }
                    }
                    if t > 16. {
                        let indices: Vec<_> = (0..codes.len()).filter(|&i| codes[i] < 3).collect();
                        let dry: Vec<_> = indices
                            .iter()
                            .map(|&i| api.read(&simplified[i].write()?))
                            .collect::<Result<_>>()?;
                        let dry = api.collection(&dry)?.coverage_simplify(t)?.polygons()?;
                        ensure!(dry.len() == indices.len(), "dry polygon count");
                        for (i, p) in indices.into_iter().zip(dry) {
                            simplified[i] = p
                        }
                    }
                    let sum = simplified
                        .iter()
                        .map(|p| p.area())
                        .collect::<Result<Vec<_>>>()?
                        .iter()
                        .sum::<f64>();
                    if (sum - tile.area()?).abs() >= 0.01 {
                        fs::write(
                            args.output.join("failed-before.wkb"),
                            api.collection(&faces)?.write()?,
                        )?;
                        fs::write(
                            args.output.join("failed-after.wkb"),
                            api.collection(&simplified)?.write()?,
                        )?;
                        fs::write(
                            args.output.join("failed-codes.json"),
                            serde_json::to_vec(&codes)?,
                        )?;
                        anyhow::bail!(
                            "Batch coverage mismatch {mode} L{l} {bx},{by}: {} -> {sum}",
                            tile.area()?
                        );
                    }
                    for (c, p) in codes.iter().zip(simplified) {
                        levels[l][*c].push(p)
                    }
                }
                println!("{mode} L{l}: vectorized strip {}/{}", by / batch + 1, n / batch);
            }
        }

        let db = Connection::open(out.join("landcover-vectors.sqlite3"))?;
        db.execute_batch("PRAGMA page_size=512;CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID;CREATE TABLE features(level INTEGER,class INTEGER,geometry BLOB,PRIMARY KEY(level,class));")?;
        let mut rows = Vec::new();
        for (l, classes) in levels.into_iter().enumerate().filter(|(l, _)| selected_level(*l)) {
            let mut features = Vec::new();
            let mut vertices = 0;
            let mut polygons = 0;
            let mut packed = 0;
            let mut area = 0.;
            let mut areas = serde_json::Map::new();
            for (c, parts) in classes.into_iter().enumerate() {
                let grouped = api
                    .collection(&parts)?
                    .union()
                    .with_context(|| format!("Global union {mode} L{l} class{c}"))?;
                ensure!(grouped.is_valid()?, "Invalid output {mode} L{l} class{c}");
                let local = grouped.transform(bounds[0], bounds[3], 1.)?;
                let a = local.area()?;
                area += a;
                areas.insert(c.to_string(), json!(a));
                let blob = compress(&local.write()?, false)?;
                packed += blob.len();
                db.execute(
                    "INSERT INTO features VALUES(?,?,?)",
                    params![l as i32, c as i32, blob],
                )?;
                let polys = local.polygons()?;
                polygons += polys.len();
                let mut coords = Vec::new();
                for p in polys {
                    let rings = p.rings()?;
                    vertices += rings.iter().map(Vec::len).sum::<usize>();
                    coords.push(rings)
                }
                features.push(json!({"type":"Feature","properties":{"class":c},"geometry":{"type":"MultiPolygon","coordinates":coords}}));
            }
            let gap = (area - side.powi(2)).abs();
            ensure!(
                gap < side.powi(2) * 1e-9,
                "Coverage area mismatch {gap}"
            );
            let bytes =
                serde_json::to_vec(&json!({"type":"FeatureCollection","features":features}))?;
            let zipped = compress(&bytes, true)?;
            let name = format!("level-{l}.geojson.gz");
            fs::write(out.join(&name), &zipped)?;
            rows.push(json!({"level":l,"tolerance":tolerances[l],"minimumArea":if args.progressive_lod{thresholds[l]as f64*res*res}else{0.},"minimumCells":if args.progressive_lod{thresholds[l]}else{args.minimum_cells},"filterRadiusCells":if args.progressive_lod{radii[l]}else{0},"changedCellsFromFine":level_changes[l],"referenceFile":format!("reference-{l}.png"),"file":name,"transferBytes":zipped.len(),"geojsonSha256":Sha256::digest(&zipped).iter().map(|b|format!("{b:02x}")).collect::<String>(),"referenceSha256":Sha256::digest(fs::read(out.join(format!("reference-{l}.png")))?).iter().map(|b|format!("{b:02x}")).collect::<String>(),"geojsonBytes":bytes.len(),"wkbZlibBytes":packed,"vertices":vertices,"polygons":polygons,"areaM2":areas,"gapAreaM2":gap}));
            println!(
                "{mode} L{l}: {polygons} polygons, {vertices} coordinates, {} bytes",
                zipped.len()
            );
        }
        base["filterBeforeVectorization"]=json!(full_resolution);
        base["filterSpanProjectedM"]=json!(args.filter_span_m);
        base["referenceGrid"]=json!(n.min(2048));
        base["waterDetailFactor"]=json!(args.water_detail);
        base["waterPixelProjectedM"]=json!(res/args.water_detail as f64);
        base["waterSimplifyM"]=json!([0.,4.,8.,16.,32.,64.]);
        if full_resolution && args.progressive_lod && high_water.is_none() {
            base["waterSimplifyM"] = json!(WATER_BANK_M);
            base["waterHoleAreaProjectedM2"] = json!(WATER_HOLE_M2);
            base["waterMinimumAreaProjectedM2"] = json!(WATER_AREA_M2);
            base["waterMinimumExtentProjectedM"] = json!(WATER_EXTENT_M);
        }
        base["progressiveLod"] = json!(args.progressive_lod);
        base["sourceGrid"] = json!(n);
        base["sourcePixelProjectedM"] = json!(res);
        base["levels"] = json!(rows);
        base["seconds"] = json!(now.elapsed().as_secs_f64());
        base["referenceBytes"] = json!(fs::metadata(out.join("reference.png"))?.len());
        base["generator"] = json!("terrain-landcover-raster / Rust");
        base["filter"] = json!({"id":mode,"label":label,"changedCells":changed,"minimumCells":args.minimum_cells,"waterProtected":true,"seaProtected":true,"ties":"retain original; ambiguous non-original ties unchanged"});
        base["notes"] = json!([
            "3方式は同じ再サンプル画像を入力とし、ベクトル化前の処理だけを変更。全方式でベクトル化後の小領域統合は行わない。",
            "分類格子はWebメルカトル62.5m。多数決は1回・3x3。小領域除去は4連結9セル未満（初期設定）。",
            "内水は増減しない。海岸線は同じGSIベクター。範囲端・海岸に接する小領域は保持。"
        ]);
        if args.progressive_lod {
            base["notes"] = json!([
                "LODごとに陸上の分類画像を整理してからベクトル化。L0の分類画像が比較基準。",
                "多数決の半径は0/1/2/4/8画素、小領域統合しきい値は9/16/64/256/1024画素。",
                "水域画素と海側画素を固定。海岸と内水のベクトル境界の共通簡略化も行わない。",
                "境界の簡略化は陸上3分類の共有辺にのみ適用。"
            ]);
            base["lodProfiles"] = json!({"radiiCells":radii,"minimumCells":thresholds});
        }
        if full_resolution {
            base["notes"]=json!([
                "元分類を保持する密度で読み、物理距離の窓で分類の多数決・面積基準の小領域除去を実行してからベクトル化。生成前の低解像度間引きは行わない。",
                "水域・海側・分類画像の外周を保護。LODの領域整理も同じ高密度分類画像上で実施。",
                "reference PNGだけはWeb表示用に処理後2048へ縮小。ベクトル生成には使用しない。"
            ]);
        }
        db.execute(
            "INSERT INTO metadata VALUES('manifest',?)",
            params![serde_json::to_string(&base)?],
        )?;
        let ok: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        ensure!(ok == "ok", "SQLite integrity");
        drop(db);
        base["sqliteBytes"] = json!(fs::metadata(out.join("landcover-vectors.sqlite3"))?.len());
        fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&base)?)?;
        results.push(json!({"id":mode,"label":label,"changedCells":changed,"seconds":now.elapsed().as_secs_f64(),"levels":rows,"sqliteBytes":base["sqliteBytes"]}));
    }
    fs::write(
        args.output.join("manifest.json"),
        serde_json::to_vec_pretty(
            &json!({"variants":results,"seconds":started.elapsed().as_secs_f64(),"size":n,"minimumCells":args.minimum_cells}),
        )?,
    )?;
    println!("Complete {:.2}s", started.elapsed().as_secs_f64());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn water_lod_omits_short_patches_but_keeps_diagonal_stream_and_coast() {
        let n = 80;
        let mut land = vec![1; n * n];
        let mut values = vec![1; n * n];
        let pond = 5 * n + 72;
        let coastal_water = 4 * n + 5;
        values[pond] = 3;
        values[coastal_water] = 3;
        land[coastal_water - 1] = 0;
        for i in 8..60 {
            values[i * n + i] = 3;
        }
        assert_eq!(prune_inland_water(&mut values, &land, n, 10., 4), 1);
        assert_eq!(values[pond], 1);
        assert_eq!(values[coastal_water], 3);
        assert!((8..60).all(|i| values[i * n + i] == 3));
    }

    #[test]
    fn sliding_window_matches_odd_integral_filter(){
        let a:Vec<_>=(0..143).map(|i|((i*31+i/9)%4)as u8).collect();let land=vec![1;143];
        for radius in [1,2,4]{assert_eq!(majority_window(&a,&land,13,11,radius*2+1),majority_radius(&a,&land,13,11,radius));}
    }
    #[test]
    fn even_window_is_symmetric_and_preserves_water(){
        let a:Vec<_>=(0..225).map(|i|((i*7+i/13)%4)as u8).collect();let land=vec![1;225];
        let b=majority_window(&a,&land,15,15,8);
        let reversed:Vec<_>=a.iter().copied().rev().collect();
        let c:Vec<_>=majority_window(&reversed,&land,15,15,8).into_iter().rev().collect();
        assert_eq!(b,c);
        for i in 0..a.len(){if a[i]==3{assert_eq!(b[i],3);}}
    }
    #[test]
    fn detailed_water_underlay_fills_only_inland_water(){
        let src=vec![1,1,0,1,3,0,1,3,2];let mask=vec![1;9];
        let dst=water_underlay(&src,&mask,3,3);
        assert!(dst.iter().all(|&c|c<3));
        for i in 0..src.len(){if src[i]!=3{assert_eq!(dst[i],src[i]);}}
    }
    #[test]
    fn integral_radius_one_matches_existing_majority() {
        let a: Vec<_> = (0..99).map(|i| ((i * 13 + i / 7) % 4) as u8).collect();
        let m: Vec<_> = (0..99).map(|i| if i % 13 == 0 { 0 } else { 1 }).collect();
        assert_eq!(majority_radius(&a, &m, 11, 9, 1), majority(&a, &m, 11, 9));
    }
    #[test]
    fn coarse_filter_protects_water_sea_and_border() {
        let mut a = vec![1; 121];
        a[60] = 3;
        a[30] = 2;
        a[0] = 2;
        let mut m = vec![1; 121];
        m[30] = 0;
        let b = majority_radius(&a, &m, 11, 11, 4);
        assert_eq!(b[60], 3);
        assert_eq!(b[30], 2);
        assert_eq!(b[0], 2);
        assert_eq!(b.iter().filter(|&&c| c == 3).count(), 1);
    }
    #[test]
    fn mode_removes_salt_and_preserves_water() {
        let mut a = vec![1; 49];
        a[24] = 2;
        a[10] = 3;
        let land = vec![1; 49];
        let b = majority(&a, &land, 7, 7);
        assert_eq!(b[24], 1);
        assert_eq!(b[10], 3);
        assert_eq!(b.iter().filter(|&&c| c == 3).count(), 1);
    }
    #[test]
    fn sea_and_perimeter_fixed() {
        let mut a = vec![1; 49];
        a[24] = 2;
        a[0] = 2;
        let mut m = vec![1; 49];
        m[24] = 0;
        let b = majority(&a, &m, 7, 7);
        assert_eq!(b[24], 2);
        assert_eq!(b[0], 2);
    }
    #[test]
    fn regions_remove_whole_component() {
        let mut a = vec![1; 49];
        a[24] = 2;
        a[25] = 2;
        a[10] = 3;
        let b = small_regions(&a, &vec![1; 49], 7, 7, 3);
        assert_eq!(b[24], 1);
        assert_eq!(b[25], 1);
        assert_eq!(b[10], 3);
    }
    #[test]
    fn combined_removes_residual_dots_and_preserves_water() {
        let mut a = vec![1; 81];
        // A 3x3 dry island survives one synchronous majority pass in a reduced form.
        for i in [30, 31, 32, 39, 40, 41, 48, 49, 50] {
            a[i] = 2;
        }
        a[12] = 3;
        let land = vec![1; 81];
        let mode = majority(&a, &land, 9, 9);
        let clean = small_regions(&mode, &land, 9, 9, 9);
        assert!(mode.iter().any(|&c| c == 2));
        assert!(!clean.iter().any(|&c| c == 2));
        assert_eq!(clean[12], 3);
        assert_eq!(clean.iter().filter(|&&c| c == 3).count(), 1);
    }
    #[test]
    fn minimum_is_strict() {
        let mut a = vec![1; 49];
        a[24] = 2;
        a[25] = 2;
        assert_eq!(small_regions(&a, &vec![1; 49], 7, 7, 2), a);
    }
    #[test]
    fn mode_tie_keeps_original() {
        let a = vec![0, 1, 1, 2, 2, 2, 0, 0, 1];
        let b = majority(&a, &vec![1; 9], 3, 3);
        assert_eq!(b[4], 2);
    }
    #[test]
    fn coastal_small_region_is_fixed() {
        let mut a = vec![1; 49];
        a[24] = 2;
        let mut land = vec![1; 49];
        land[23] = 0;
        assert_eq!(small_regions(&a, &land, 7, 7, 9)[24], 2);
    }
    #[test]
    fn mode_is_invariant_to_dry_class_numbering() {
        let a = vec![0, 1, 1, 0, 2, 1, 0, 1, 2];
        let p = [2, 0, 1, 3];
        let changed: Vec<_> = a.iter().map(|&v| p[v as usize]).collect();
        let b = majority(&a, &vec![1; 9], 3, 3);
        let c = majority(&changed, &vec![1; 9], 3, 3);
        assert_eq!(c, b.iter().map(|&v| p[v as usize]).collect::<Vec<_>>());
    }
    #[test]
    fn classification_ids_have_no_order() {
        assert_eq!(classify(10), 1);
        assert_eq!(classify(60), 2);
        assert_eq!(classify(80), 3);
        assert_eq!(classify(50), 0);
    }
}
