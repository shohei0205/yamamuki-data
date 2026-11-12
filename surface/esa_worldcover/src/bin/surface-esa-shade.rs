#[path = "../dem_codec.rs"]
mod codec;
#[path = "../hillshade.rs"]
mod hillshade;
#[path = "../shade_compress.rs"]
mod shade_compress;

use anyhow::Result;

fn main() -> Result<()> {
    match std::env::args_os()
        .nth(1)
        .as_deref()
        .and_then(|arg| arg.to_str())
    {
        Some("--compress-hillshade") => shade_compress::run(),
        Some("--pack-ground-512") => hillshade::pack_ground(),
        Some("--merge-hillshade") => hillshade::merge(),
        _ => hillshade::run(),
    }
}
