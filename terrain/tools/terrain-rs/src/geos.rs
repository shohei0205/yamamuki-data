//! GEOS の C API を直接使う。Python と Shapely の実行環境には依存しない。
use anyhow::{Result, ensure};
use std::{ffi::c_void, path::Path};
type Ptr = *mut c_void;
type Unary = unsafe extern "C" fn(Ptr, Ptr) -> Ptr;
type Binary = unsafe extern "C" fn(Ptr, Ptr, Ptr) -> Ptr;
pub struct Geos {
    library: libloading::Library,
    context: Ptr,
}
pub struct Geometry<'a> {
    api: &'a Geos,
    pointer: Ptr,
}
impl Geos {
    pub fn open(path: &Path) -> Result<Self> {
        unsafe {
            let library = libloading::Library::new(path)?;
            let init: libloading::Symbol<unsafe extern "C" fn() -> Ptr> =
                library.get(b"GEOS_init_r")?;
            let context = init();
            ensure!(!context.is_null(), "GEOS の初期化に失敗しました");
            Ok(Self { library, context })
        }
    }
    fn owned(&self, pointer: Ptr) -> Result<Geometry<'_>> {
        ensure!(!pointer.is_null(), "GEOS の地形処理に失敗しました");
        Ok(Geometry { api: self, pointer })
    }
    pub fn read(&self, bytes: &[u8]) -> Result<Geometry<'_>> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(Ptr, *const u8, usize) -> Ptr> =
                self.library.get(b"GEOSGeomFromWKB_buf_r")?;
            self.owned(f(self.context, bytes.as_ptr(), bytes.len()))
        }
    }
    pub fn rectangle(&self, left: f64, bottom: f64, right: f64, top: f64) -> Result<Geometry<'_>> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(Ptr, f64, f64, f64, f64) -> Ptr> =
                self.library.get(b"GEOSGeom_createRectangle_r")?;
            self.owned(f(self.context, left, bottom, right, top))
        }
    }
    pub fn polygon(&self, points: &[[f64; 2]]) -> Result<Geometry<'_>> {
        ensure!(points.len() >= 3, "輪郭の点が不足しています");
        unsafe {
            let create: libloading::Symbol<unsafe extern "C" fn(Ptr, u32, u32) -> Ptr> =
                self.library.get(b"GEOSCoordSeq_create_r")?;
            let set: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, u32, f64, f64) -> i32> =
                self.library.get(b"GEOSCoordSeq_setXY_r")?;
            let ring: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr) -> Ptr> =
                self.library.get(b"GEOSGeom_createLinearRing_r")?;
            let polygon: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, *mut Ptr, u32) -> Ptr> =
                self.library.get(b"GEOSGeom_createPolygon_r")?;
            let sequence = create(self.context, (points.len() + 1) as u32, 2);
            ensure!(!sequence.is_null(), "輪郭の確保に失敗しました");
            for (i, p) in points.iter().chain(points.first()).enumerate() {
                ensure!(
                    set(self.context, sequence, i as u32, p[0], p[1]) == 1,
                    "輪郭の座標が不正です"
                );
            }
            let shell = ring(self.context, sequence);
            ensure!(!shell.is_null(), "輪郭の生成に失敗しました");
            self.owned(polygon(self.context, shell, std::ptr::null_mut(), 0))
        }
    }
    pub fn collection<'a>(&'a self, parts: &[Geometry<'a>]) -> Result<Geometry<'a>> {
        unsafe {
            let clone: libloading::Symbol<Unary> = self.library.get(b"GEOSGeom_clone_r")?;
            let mut pointers: Vec<_> = parts
                .iter()
                .map(|g| clone(self.context, g.pointer))
                .collect();
            ensure!(
                pointers.iter().all(|p| !p.is_null()),
                "輪郭の複製に失敗しました"
            );
            let create: libloading::Symbol<unsafe extern "C" fn(Ptr, i32, *mut Ptr, u32) -> Ptr> =
                self.library.get(b"GEOSGeom_createCollection_r")?;
            self.owned(create(
                self.context,
                7,
                pointers.as_mut_ptr(),
                pointers.len() as u32,
            ))
        }
    }
}
impl Drop for Geos {
    fn drop(&mut self) {
        unsafe {
            if let Ok(f) = self
                .library
                .get::<unsafe extern "C" fn(Ptr)>(b"GEOS_finish_r")
            {
                f(self.context)
            }
        }
    }
}
impl Drop for Geometry<'_> {
    fn drop(&mut self) {
        unsafe {
            if let Ok(f) = self
                .api
                .library
                .get::<unsafe extern "C" fn(Ptr, Ptr)>(b"GEOSGeom_destroy_r")
            {
                f(self.api.context, self.pointer)
            }
        }
    }
}
impl<'a> Geometry<'a> {
    fn unary(&self, name: &[u8]) -> Result<Self> {
        unsafe {
            let f: libloading::Symbol<Unary> = self.api.library.get(name)?;
            self.api.owned(f(self.api.context, self.pointer))
        }
    }
    fn binary(&self, other: &Self, name: &[u8]) -> Result<Self> {
        ensure!(
            std::ptr::eq(self.api, other.api),
            "異なる GEOS の輪郭は混ぜられません"
        );
        unsafe {
            let f: libloading::Symbol<Binary> = self.api.library.get(name)?;
            self.api
                .owned(f(self.api.context, self.pointer, other.pointer))
        }
    }
    fn predicate(&self, other: Option<&Self>, name: &[u8]) -> Result<bool> {
        unsafe {
            let answer = if let Some(other) = other {
                ensure!(
                    std::ptr::eq(self.api, other.api),
                    "異なる GEOS の輪郭は混ぜられません"
                );
                let f: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, Ptr) -> i8> =
                    self.api.library.get(name)?;
                f(self.api.context, self.pointer, other.pointer)
            } else {
                let f: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr) -> i8> =
                    self.api.library.get(name)?;
                f(self.api.context, self.pointer)
            };
            ensure!(answer == 0 || answer == 1, "GEOS の判定に失敗しました");
            Ok(answer == 1)
        }
    }
    pub fn is_valid(&self) -> Result<bool> {
        self.predicate(None, b"GEOSisValid_r")
    }
    pub fn is_empty(&self) -> Result<bool> {
        self.predicate(None, b"GEOSisEmpty_r")
    }
    pub fn covers(&self, other: &Self) -> Result<bool> {
        self.predicate(Some(other), b"GEOSCovers_r")
    }
    pub fn intersects(&self, other: &Self) -> Result<bool> {
        self.predicate(Some(other), b"GEOSIntersects_r")
    }
    pub fn intersection(&self, other: &Self) -> Result<Self> {
        self.binary(other, b"GEOSIntersection_r")
    }
    pub fn difference(&self, other: &Self) -> Result<Self> {
        self.binary(other, b"GEOSDifference_r")
    }
    pub fn union(&self) -> Result<Self> {
        self.unary(b"GEOSUnaryUnion_r")
    }
    pub fn triangulate(&self) -> Result<Self> {
        self.unary(b"GEOSConstrainedDelaunayTriangulation_r")
    }
    pub fn minimum_rectangle(&self) -> Result<Self> {
        self.unary(b"GEOSMinimumRotatedRectangle_r")
    }
    pub fn precision(&self, grid: f64) -> Result<Self> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, f64, i32) -> Ptr> =
                self.api.library.get(b"GEOSGeom_setPrecision_r")?;
            self.api.owned(f(self.api.context, self.pointer, grid, 0))
        }
    }
    pub fn area(&self) -> Result<f64> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, *mut f64) -> i32> =
                self.api.library.get(b"GEOSArea_r")?;
            let mut area = 0.;
            ensure!(
                f(self.api.context, self.pointer, &mut area) == 1,
                "面積を取得できません"
            );
            Ok(area)
        }
    }
    pub fn write(&self) -> Result<Vec<u8>> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, *mut usize) -> *mut u8> =
                self.api.library.get(b"GEOSGeomToWKB_buf_r")?;
            let free: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr)> =
                self.api.library.get(b"GEOSFree_r")?;
            let mut length = 0;
            let bytes = f(self.api.context, self.pointer, &mut length);
            ensure!(!bytes.is_null(), "輪郭の保存に失敗しました");
            let result = std::slice::from_raw_parts(bytes, length).to_vec();
            free(self.api.context, bytes.cast());
            Ok(result)
        }
    }
    pub fn simplify(&self, tolerance: f64) -> Result<Self> {
        unsafe {
            ensure!(
                tolerance.is_finite() && tolerance >= 0.,
                "輪郭の誤差上限が不正です"
            );
            let parts = self.polygons()?;
            let before = self.counts()?;
            let collection = self.api.collection(&parts)?;
            let f: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, f64, i32) -> Ptr> =
                self.api.library.get(b"GEOSCoverageSimplifyVW_r")?;
            let result = self
                .api
                .owned(f(self.api.context, collection.pointer, tolerance, 0))?
                .union()?;
            ensure!(
                result.is_valid()? && result.counts()? == before,
                "簡略化で島や穴の数が変わりました"
            );
            Ok(result)
        }
    }
    fn kind(&self) -> Result<i32> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr) -> i32> =
                self.api.library.get(b"GEOSGeomTypeId_r")?;
            let kind = f(self.api.context, self.pointer);
            ensure!(kind >= 0, "輪郭の種類を取得できません");
            Ok(kind)
        }
    }
    pub fn polygons(&self) -> Result<Vec<Self>> {
        unsafe {
            let clone: libloading::Symbol<Unary> = self.api.library.get(b"GEOSGeom_clone_r")?;
            match self.kind()? {
                3 => Ok(vec![self.api.owned(clone(self.api.context, self.pointer))?]),
                6 | 7 => {
                    let count: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr) -> i32> =
                        self.api.library.get(b"GEOSGetNumGeometries_r")?;
                    let get: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, i32) -> Ptr> =
                        self.api.library.get(b"GEOSGetGeometryN_r")?;
                    let count = count(self.api.context, self.pointer);
                    ensure!(count >= 0, "輪郭の個数が不正です");
                    let mut parts = Vec::new();
                    for i in 0..count {
                        let child = self.api.owned(clone(
                            self.api.context,
                            get(self.api.context, self.pointer, i),
                        ))?;
                        parts.extend(child.polygons()?);
                    }
                    Ok(parts)
                }
                _ => Ok(Vec::new()),
            }
        }
    }
    pub fn counts(&self) -> Result<(usize, usize)> {
        unsafe {
            let parts = self.polygons()?;
            let f: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr) -> i32> =
                self.api.library.get(b"GEOSGetNumInteriorRings_r")?;
            let mut holes = 0;
            for p in &parts {
                let n = f(self.api.context, p.pointer);
                ensure!(n >= 0, "穴の数を取得できません");
                holes += n as usize;
            }
            Ok((parts.len(), holes))
        }
    }
    pub fn rings(&self) -> Result<Vec<Vec<[f64; 2]>>> {
        unsafe {
            let count: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr) -> i32> =
                self.api.library.get(b"GEOSGetNumInteriorRings_r")?;
            let outer: libloading::Symbol<Unary> =
                self.api.library.get(b"GEOSGetExteriorRing_r")?;
            let inner: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, i32) -> Ptr> =
                self.api.library.get(b"GEOSGetInteriorRingN_r")?;
            let sequence: libloading::Symbol<Unary> =
                self.api.library.get(b"GEOSGeom_getCoordSeq_r")?;
            let size: libloading::Symbol<unsafe extern "C" fn(Ptr, Ptr, *mut u32) -> i32> =
                self.api.library.get(b"GEOSCoordSeq_getSize_r")?;
            let get: libloading::Symbol<
                unsafe extern "C" fn(Ptr, Ptr, u32, *mut f64, *mut f64) -> i32,
            > = self.api.library.get(b"GEOSCoordSeq_getXY_r")?;
            let mut result = Vec::new();
            for p in self.polygons()? {
                let n = count(self.api.context, p.pointer);
                ensure!(n >= 0, "穴の数が不正です");
                for i in -1..n {
                    let ring = if i < 0 {
                        outer(self.api.context, p.pointer)
                    } else {
                        inner(self.api.context, p.pointer, i)
                    };
                    ensure!(!ring.is_null(), "輪郭を取得できません");
                    let seq = sequence(self.api.context, ring);
                    let mut length = 0;
                    ensure!(
                        size(self.api.context, seq, &mut length) == 1,
                        "輪郭の長さが不正です"
                    );
                    let mut coords = Vec::new();
                    for j in 0..length {
                        let mut xy = [0., 0.];
                        let [x, y] = &mut xy;
                        ensure!(
                            get(self.api.context, seq, j, x, y) == 1,
                            "輪郭の座標が不正です"
                        );
                        coords.push(xy);
                    }
                    result.push(coords);
                }
            }
            Ok(result)
        }
    }
    pub fn transform(&self, left: f64, top: f64, resolution: f64) -> Result<Self> {
        unsafe extern "C" fn callback(x: *mut f64, y: *mut f64, data: Ptr) -> i32 {
            unsafe {
                let [left, top, res] = *(data as *const [f64; 3]);
                *x = (*x - left) / res;
                *y = (top - *y) / res;
            }
            1
        }
        ensure!(
            resolution > 0. && resolution.is_finite(),
            "輪郭の縮尺が不正です"
        );
        unsafe {
            let f: libloading::Symbol<
                unsafe extern "C" fn(
                    Ptr,
                    Ptr,
                    unsafe extern "C" fn(*mut f64, *mut f64, Ptr) -> i32,
                    Ptr,
                ) -> Ptr,
            > = self.api.library.get(b"GEOSGeom_transformXY_r")?;
            let mut data = [left, top, resolution];
            self.api.owned(f(
                self.api.context,
                self.pointer,
                callback,
                (&mut data as *mut [f64; 3]).cast(),
            ))
        }
    }
}
