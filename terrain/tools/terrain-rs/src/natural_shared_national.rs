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
    BenchmarkLevels,
    BenchmarkFitCache {
        #[arg(long)] snapshot:PathBuf,
        #[arg(long)] output:PathBuf,
        #[arg(long,default_value_t=32)] batches:usize,
        #[arg(long,default_value_t=1000)] offset:usize,
    },
    BenchmarkAudit {
        #[arg(long)] snapshot:PathBuf,
        #[arg(long)] output:PathBuf,
        #[arg(long,default_value_t=256)] count:usize,
        #[arg(long,default_value_t=50000)] offset:usize,
    },
    BenchmarkPrune {
        #[arg(long)] snapshot: PathBuf,
        #[arg(long)] output: PathBuf,
        #[arg(long, default_value_t=2000)] candidates: usize,
        #[arg(long, default_value_t=0)] skip_per_stripe: usize,
        #[arg(long, default_value_t=13)] z: u8,
    },
    Benchmark {
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 16)]
        batches: usize,
        #[arg(long, default_value_t = 10)]
        workers: usize,
        #[arg(long, default_value_t = 0)]
        start_batch: usize,
        #[arg(long)]
        serial_prepare: bool,
    },
    Profile {
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long, default_value_t = 16)]
        count: usize,
    },
}
fn main() -> Result<()> {
    let a = Args::parse();
    if let Command::BenchmarkFitCache{snapshot,output,batches,offset}=&a.command {
        return natural_shared_pipeline::benchmark_fit_cache(&a.prepared,snapshot,output,*batches,*offset);
    }
    if let Command::BenchmarkAudit{snapshot,output,count,offset}=&a.command {
        return natural_shared_pipeline::benchmark_audit(&a.prepared,snapshot,output,*offset,*count);
    }
    if let Command::BenchmarkPrune { snapshot, output, candidates, skip_per_stripe, z } = &a.command {
        return natural_shared_pipeline::benchmark_prune(&a.prepared,snapshot,output,*candidates,*skip_per_stripe,*z);
    }
    if let Command::Profile { snapshot, count } = &a.command {
        return natural_shared_pipeline::profile(&a.prepared, snapshot, *count);
    }
    if let Command::Benchmark {
        snapshot,
        output,
        batches,
        workers,
        start_batch,
        serial_prepare,
    } = &a.command
    {
        return natural_shared_pipeline::benchmark(
            &a.prepared,
            snapshot,
            output,
            *batches,
            *workers,
            *start_batch,
            *serial_prepare,
        );
    }
    if matches!(&a.command, Command::BenchmarkLevels) {
        return natural_shared_pipeline::benchmark_levels(&a.prepared);
    }
    natural_shared_pipeline::execute(
        &a.prepared,
        a.local_point_limit,
        match &a.command {
            Command::Init => "init",
            Command::Run { .. } => "run",
            Command::Verify => "verify",
            Command::BenchmarkFitCache{..} | Command::BenchmarkAudit{..} | Command::Profile { .. } | Command::Benchmark { .. } | Command::BenchmarkPrune { .. } | Command::BenchmarkLevels => {
                unreachable!()
            }
        },
        match a.command {
            Command::Run { window } => window,
            _ => Vec::new(),
        },
    )
}
