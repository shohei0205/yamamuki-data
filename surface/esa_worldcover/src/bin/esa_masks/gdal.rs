//! 同梱GDALでWorldCoverの再投影と陸域のラスタ化を行う。
use anyhow::{Context, Result, ensure};
use gdal_src as _;
use gdal_sys::*;
use std::{
    ffi::{CStr, CString, c_void},
    fs,
    mem::ManuallyDrop,
    path::Path,
    ptr::{NonNull, null_mut},
};

pub struct Dataset(NonNull<c_void>);

impl Dataset {
    fn new(handle: GDALDatasetH, operation: &str) -> Result<Self> {
        NonNull::new(handle)
            .map(Self)
            .with_context(|| gdal_error(operation))
    }

    fn open(path: &Path) -> Result<Self> {
        let name = path_string(path)?;
        let handle = unsafe { GDALOpen(name.as_ptr(), GDALAccess::GA_ReadOnly) };
        Self::new(handle, &format!("VRTを開く {}", path.display()))
    }

    fn close(self) -> Result<()> {
        let dataset = ManuallyDrop::new(self);
        check(unsafe { GDALClose(dataset.0.as_ptr()) }, "VRTの書き出し")
    }

    pub fn read(&self, x: usize, y: usize, side: usize, data: &mut [u8]) -> Result<()> {
        let (x, y, side) = (i32::try_from(x)?, i32::try_from(y)?, i32::try_from(side)?);
        ensure!(
            data.len() == side as usize * side as usize,
            "バッファサイズが不正"
        );
        let band = unsafe { GDALGetRasterBand(self.0.as_ptr(), 1) };
        ensure!(!band.is_null(), "{}", gdal_error("ラスタバンドを取得"));
        check(
            unsafe {
                GDALRasterIO(
                    band,
                    GDALRWFlag::GF_Read,
                    x,
                    y,
                    side,
                    side,
                    data.as_mut_ptr().cast(),
                    side,
                    side,
                    GDALDataType::GDT_Byte,
                    0,
                    0,
                )
            },
            "ラスタの読み込み",
        )
    }
}

impl Drop for Dataset {
    fn drop(&mut self) {
        unsafe { GDALClose(self.0.as_ptr()) };
    }
}

fn gdal_error(operation: &str) -> String {
    let message = unsafe { CStr::from_ptr(CPLGetLastErrorMsg()) };
    format!("{operation}: {}", message.to_string_lossy())
}

fn check(status: CPLErr::Type, operation: &str) -> Result<()> {
    ensure!(status == CPLErr::CE_None, "{}", gdal_error(operation));
    Ok(())
}

fn path_string(path: &Path) -> Result<CString> {
    let absolute = std::path::absolute(path)?;
    let utf8 = absolute.to_str().context("パスをUTF-8に変換できません")?;
    Ok(CString::new(utf8)?)
}

pub fn project_worldcover(
    directory: &Path,
    output: &Path,
    extent: [f64; 4],
    width: usize,
    height: usize,
) -> Result<Dataset> {
    unsafe { GDALAllRegister() };
    let mut sources = Vec::new();
    for entry in fs::read_dir(directory)
        .with_context(|| format!("WorldCover原本フォルダー {}", directory.display()))?
    {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("ESA_WorldCover_10m_2021_v200_")
            && name.ends_with("_Map.tif")
            && path.is_file()
        {
            sources.push(path);
        }
    }
    ensure!(
        !sources.is_empty(),
        "ESA WorldCover 2021 v200のGeoTIFFがありません"
    );
    sources.sort_unstable();
    let names = sources
        .iter()
        .map(|path| path_string(path))
        .collect::<Result<Vec<_>>>()?;
    let names_ptr = names.iter().map(|name| name.as_ptr()).collect::<Vec<_>>();
    let count = i32::try_from(names.len())?;
    let mosaic_path = output.join("worldcover.vrt");
    let mosaic_name = path_string(&mosaic_path)?;
    let mut mosaic_args = [
        c"-srcnodata".as_ptr().cast_mut(),
        c"0".as_ptr().cast_mut(),
        c"-vrtnodata".as_ptr().cast_mut(),
        c"0".as_ptr().cast_mut(),
        null_mut(),
    ];
    let options = unsafe { GDALBuildVRTOptionsNew(mosaic_args.as_mut_ptr(), null_mut()) };
    ensure!(!options.is_null(), "{}", gdal_error("モザイク設定"));
    let mut usage_error = 0;
    let mosaic_handle = unsafe {
        let handle = GDALBuildVRT(
            mosaic_name.as_ptr(),
            count,
            null_mut(),
            names_ptr.as_ptr(),
            options,
            &mut usage_error,
        );
        GDALBuildVRTOptionsFree(options);
        handle
    };
    Dataset::new(mosaic_handle, "WorldCoverのモザイク作成")?.close()?;
    let mosaic = Dataset::open(&mosaic_path)?;

    let projected_path = output.join("source-3857.vrt");
    let projected_name = path_string(&projected_path)?;
    let warp_args = [
        "-of",
        "VRT",
        "-t_srs",
        "EPSG:3857",
        "-r",
        "near",
        "-et",
        "0",
        "-srcnodata",
        "0",
        "-dstnodata",
        "0",
        "-te",
    ];
    let args = warp_args
        .into_iter()
        .map(str::to_owned)
        .chain(extent.map(|value| value.to_string()))
        .chain(["-ts".to_owned(), width.to_string(), height.to_string()]);
    let args = args
        .map(CString::new)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut argv = args
        .iter()
        .map(|arg| arg.as_ptr().cast_mut())
        .chain([null_mut()])
        .collect::<Vec<_>>();
    let options = unsafe { GDALWarpAppOptionsNew(argv.as_mut_ptr(), null_mut()) };
    ensure!(!options.is_null(), "{}", gdal_error("再投影設定"));
    let mut source = mosaic.0.as_ptr();
    let projected_handle = unsafe {
        let handle = GDALWarp(
            projected_name.as_ptr(),
            null_mut(),
            1,
            &mut source,
            options,
            &mut usage_error,
        );
        GDALWarpAppOptionsFree(options);
        handle
    };
    Dataset::new(projected_handle, "WorldCoverのEPSG:3857への再投影")?.close()?;
    drop(mosaic);
    Dataset::open(&projected_path)
}
