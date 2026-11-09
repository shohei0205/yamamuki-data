use anyhow::Result;

#[path = "../dem_codec.rs"]
mod codec;
#[path = "esa_masks/gdal.rs"]
mod gdal;
#[path = "esa_masks/raster.rs"]
mod raster;

fn main() -> Result<()> {
    raster::run()
}

fn classify(v: u8) -> u8 {
    match v {
        10 | 95 => 1,
        60 => 2,
        80 => 3,
        _ => 0,
    }
}
