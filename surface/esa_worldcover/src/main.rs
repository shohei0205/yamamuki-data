mod acquire;
mod dem;
mod dem_codec;
mod pack;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::Path;
use surface_tool::write_database;

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        return usage();
    };
    match (command, &args[1..]) {
        ("plan-worldcover", _) => {
            acquire::worldcover::run(acquire::worldcover::Args::parse_from(&args))
        }
        ("plan-surface", [input]) => acquire::plan_surface(Path::new(input)),
        ("prepare-dem", [input, output]) => write_database(Path::new(output), false, |db| {
            dem::build(db, Path::new(input))
        }),
        ("pack-esa", [input, output, version]) => {
            let version = version
                .to_str()
                .filter(|version| !version.trim().is_empty())
                .context("データ版を指定してください")?;
            write_database(Path::new(output), true, |db| {
                pack::import(db, Path::new(input), version)
            })
        }
        _ => usage(),
    }
}

fn usage() -> Result<()> {
    anyhow::bail!(
        "使い方: surface-esa-worldcover prepare-dem <z14のDEM PNGフォルダー> <出力> | plan-worldcover <作業用標高SQLite> [--halo-tiles <数>] | plan-surface <作業用標高SQLite> | pack-esa <生成済みの中間SQLite> <出力> <データ版>"
    );
}
