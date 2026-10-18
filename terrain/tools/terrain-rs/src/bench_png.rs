use anyhow::{Result, ensure};
use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::Path,
};
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[u8; 4]>,
}
pub fn read(path: &Path) -> Result<Image> {
    let mut decoder = png::Decoder::new(BufReader::new(File::open(path)?));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| anyhow::anyhow!("PNG が大きすぎます"))?
    ];
    let info = reader.next_frame(&mut buffer)?;
    let channels = info.color_type.samples();
    let mut pixels = Vec::new();
    for p in buffer[..info.buffer_size()].chunks_exact(channels) {
        pixels.push(match info.color_type {
            png::ColorType::Rgb => [p[0], p[1], p[2], 255],
            png::ColorType::Rgba => [p[0], p[1], p[2], p[3]],
            png::ColorType::Grayscale => [p[0], p[0], p[0], 255],
            _ => anyhow::bail!("比較用 PNG は RGB/RGBA が必要です"),
        });
    }
    Ok(Image {
        width: info.width as usize,
        height: info.height as usize,
        pixels,
    })
}
pub fn write(path: &Path, width: usize, height: usize, pixels: &[[u8; 4]]) -> Result<()> {
    write_compressed(path, width, height, pixels, png::Compression::High)
}
pub fn write_encoder_input(
    path: &Path,
    width: usize,
    height: usize,
    pixels: &[[u8; 4]],
) -> Result<()> {
    // 外部エンコーダーの PNG 復号器との互換性を保つため、中間画像は圧縮しない。
    write_compressed(path, width, height, pixels, png::Compression::NoCompression)
}
fn write_compressed(
    path: &Path,
    width: usize,
    height: usize,
    pixels: &[[u8; 4]],
    compression: png::Compression,
) -> Result<()> {
    ensure!(pixels.len() == width * height, "PNG の画素数が不正です");
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(path)?),
        width as u32,
        height as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(compression);
    let bytes: Vec<u8> = pixels.iter().flat_map(|p| p.iter().copied()).collect();
    encoder.write_header()?.write_image_data(&bytes)?;
    Ok(())
}
pub fn heights(root: &Path, level: u32) -> Result<(serde_json::Value, Vec<f32>)> {
    let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join(format!("levels/{level}/manifest.json")),
    )?)?;
    let image = read(&root.join(format!("levels/{level}/height.png")))?;
    let offset = meta["heightOffset"].as_f64().unwrap() as f32;
    ensure!(
        image.width == meta["width"].as_u64().unwrap() as usize
            && image.height == meta["height"].as_u64().unwrap() as usize,
        "標高 PNG の寸法が一致しません"
    );
    let values = image
        .pixels
        .iter()
        .map(|p| {
            if p[2] > 0 {
                p[0] as f32 + 256. * p[1] as f32 + offset
            } else {
                f32::NAN
            }
        })
        .collect();
    Ok((meta, values))
}
