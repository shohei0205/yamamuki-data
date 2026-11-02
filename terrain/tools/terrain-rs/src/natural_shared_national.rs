//! 全国共通点集合の生成・監査・保存。旧LPF経路と分離する。
#[allow(dead_code)]
mod natural_codec;
#[allow(dead_code)]
mod natural_inventory;
mod natural_local_gradient;
#[allow(dead_code)]
mod natural_regional_surface;
mod natural_shared_pipeline;
mod natural_shared_resources;
mod natural_shared_store;
#[allow(dead_code)]
mod natural_surface;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
#[derive(Parser)]
struct Args {
    #[arg(long)]
    prepared: PathBuf,
    #[arg(long, default_value_t = 131072)]
    local_point_limit: usize,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Init,
    /// 検証用に入力範囲を絞る。全国の完了と区別する。
    Run {
        #[arg(long, num_args = 4)]
        window: Vec<u32>,
    },
    Verify,
}
fn main() -> Result<()> {
    let a = Args::parse();
    natural_shared_pipeline::execute(
        &a.prepared,
        a.local_point_limit,
        match &a.command {
            Command::Init => "init",
            Command::Run { .. } => "run",
            Command::Verify => "verify",
        },
        match a.command {
            Command::Run { window } => window,
            _ => Vec::new(),
        },
    )
}
