use std::{collections::VecDeque, env, future::Future, path::Path};

use chrono::prelude::*;
use cid::Cid;
use forest_actor_interface::EPOCHS_IN_DAY;
use forest_blocks::{BlockHeader, Tipset, TipsetKeys};
use forest_db::{db_engine::open_db, parity_db::ParityDb, Store};
use forest_ipld::{recurse_links_hash, CidHashSet};
use forest_utils::db::BlockstoreExt;
use fvm_ipld_encoding::Cbor;
use memory_stats::memory_stats;
use tracing::info;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    forest_shim::address::set_current_network(forest_shim::address::Network::Testnet);

    if let Some(usage) = memory_stats() {
        info!("Current physical memory usage: {}", usage.physical_mem);
        info!("Current virtual memory usage: {}", usage.virtual_mem);
    }

    let db_path = format!("{}/.local/share/forest/calibnet/paritydb", env!("HOME"));
    let db_path = Path::new(&db_path);
    info!("db_path: {}", db_path.display());
    let db = open_db(db_path, &Default::default())?;
    let tipset = load_heaviest_tipset(&db)?;

    if let Some(usage) = memory_stats() {
        info!("Current physical memory usage: {}", usage.physical_mem);
        info!("Current virtual memory usage: {}", usage.virtual_mem);
    }

    info!("tipset epoch: {}", tipset.epoch());

    // let start = Utc::now();
    // info!("Interating DB...");
    // db.db.iter_column_while(0, |is| true)?;
    // info!(
    //     "Done interating DB finished, took {}s",
    //     (Utc::now() - start).num_seconds()
    // );

    let seen = walk_snapshot(&tipset, |cid| {
        let db_cloned = db.clone();
        async move {
            let block = db_cloned
                .get_obj(&cid)?
                .ok_or_else(|| anyhow::anyhow!("Cid {cid} not found"))?;
            Ok(block)
        }
    })
    .await
    .unwrap();

    info!("seen: {}", seen.inner().len());

    Ok(())
}

fn load_heaviest_tipset(db: &ParityDb) -> anyhow::Result<Tipset> {
    let tipset_keys_bytes = db.read("head")?.ok_or(anyhow::anyhow!("head not found"))?;
    let tipset_keys: TipsetKeys = fvm_ipld_encoding::from_slice(&tipset_keys_bytes)?;
    let block_headers: Vec<BlockHeader> = tipset_keys
        .cids()
        .iter()
        .map(|c| db.get_obj(c).unwrap().unwrap())
        .collect();
    Ok(Tipset::new(block_headers)?)
}

pub async fn walk_snapshot<F, T>(tipset: &Tipset, mut load_block: F) -> anyhow::Result<CidHashSet>
where
    F: FnMut(Cid) -> T + Send,
    T: Future<Output = Result<Vec<u8>, anyhow::Error>> + Send,
{
    let mut seen = CidHashSet::default();
    let mut blocks_to_walk: VecDeque<Cid> = tipset.cids().to_vec().into();
    let mut current_min_height = tipset.epoch();
    let incl_roots_epoch = tipset.epoch() - 2000;

    while let Some(next) = blocks_to_walk.pop_front() {
        if !seen.insert(&next) {
            continue;
        }

        let data = load_block(next).await?;

        let h = BlockHeader::unmarshal_cbor(&data)?;

        if current_min_height > h.epoch() {
            current_min_height = h.epoch();
            if current_min_height % EPOCHS_IN_DAY == 0 {
                info!(target: "chain_api", "export at: {}", current_min_height);
            }
        }

        if h.epoch() > incl_roots_epoch {
            recurse_links_hash(&mut seen, *h.messages(), &mut load_block).await?;
        }

        if h.epoch() > 0 {
            for p in h.parents().cids() {
                blocks_to_walk.push_back(*p);
            }
        } else {
            for p in h.parents().cids() {
                load_block(*p).await?;
            }
        }

        if h.epoch() == 0 || h.epoch() > incl_roots_epoch {
            recurse_links_hash(&mut seen, *h.state_root(), &mut load_block).await?;
        }
    }

    Ok(seen)
}
