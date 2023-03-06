// Copyright 2019-2023 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::path::{Path, PathBuf};

use clap::Parser;
use forest_ipld::CidHashSet;
use fvm_ipld_car::CarReader;
use tokio::io::BufReader;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::info;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
struct Opts {
    pub snapshot1: PathBuf,
    pub snapshot2: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let opts = Opts::parse();

    let mut cids = CidHashSet::default();
    load_car(&opts.snapshot1, &mut cids).await?;
    let cids1 = cids.clone();
    cids.inner_mut().clear();
    load_car(&opts.snapshot2, &mut cids).await?;
    let cids2 = cids;

    let mut common = 0;
    cids1.inner().iter().for_each(|cid| {
        if cids2.inner().contains(cid) {
            common += 1;
        }
    });

    info!("Common cids: {common}");

    Ok(())
}

async fn load_car(path: &Path, cids: &mut CidHashSet) -> anyhow::Result<()> {
    info!("Loading car file {}", path.display());
    let file = tokio::fs::File::open(path).await?;
    let reader = BufReader::new(file);
    let mut car_reader = CarReader::new(reader.compat()).await?;
    while let Some(block) = car_reader.next_block().await? {
        cids.insert(&block.cid);
    }
    info!("Loaded {} cids", cids.inner().len());
    Ok(())
}
