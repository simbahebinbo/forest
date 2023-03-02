use std::{
    collections::VecDeque,
    env,
    future::Future,
    path::Path,
    sync::{
        atomic::{self, AtomicBool, AtomicUsize},
        Arc,
    },
    time::Duration,
};

use chrono::prelude::*;
use cid::Cid;
use forest_actor_interface::EPOCHS_IN_DAY;
use forest_blocks::{BlockHeader, Tipset, TipsetKeys};
use forest_db::{db_engine::open_db, parity_db::ParityDb, Store};
use forest_ipld::{recurse_links_hash, CidHashSet};
use forest_utils::db::BlockstoreExt;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::Cbor;
use human_repr::HumanCount;
use memory_stats::memory_stats;
use tempfile::TempDir;
use tracing::*;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let mem_stats_tracker = MemStatsTracker::default();
    mem_stats_tracker.run_async();

    let db_path_raw = format!("{}/.local/share/forest/calibnet/paritydb", env!("HOME"));
    let db_path_raw = Path::new(&db_path_raw);

    let db_path = TempDir::new()?;
    fs_extra::dir::copy(db_path_raw, db_path.path(), &Default::default())?;

    mark_and_sweep(db_path).await?;

    Ok(())
}

async fn mark_and_sweep(db_path: TempDir) -> anyhow::Result<()> {
    print_db_stats(db_path.path());

    let db = open_db(
        db_path.path().join("paritydb").as_path(),
        &Default::default(),
    )?;
    let tipset = load_heaviest_tipset(&db)?;

    info!("tipset epoch: {}", tipset.epoch());

    let start = Utc::now();
    info!("Walking snapshot...");
    let seen = walk_snapshot(&tipset, |cid| {
        let db_cloned = db.clone();
        async move {
            let block = db_cloned
                .get(&cid)?
                .ok_or_else(|| anyhow::anyhow!("Cid {cid} not found"))?;
            Ok(block)
        }
    })
    .await?;
    info!(
        "Done Walking snapshot, seen: {}, took {}s",
        seen.inner().len(),
        (Utc::now() - start).num_seconds()
    );

    let start = Utc::now();
    info!("Cleaning DB...");
    let mut deleted = 0;
    db.db.iter_column_while(0, |is| {
        if let Ok(cid) = Cid::read_bytes(is.key.as_slice()) {
            if !seen.contains(&cid) {
                if db.delete(is.key).is_ok() {
                    deleted += 1;
                } else {
                    warn!("Error deleting cid {cid}");
                }
            }
        }

        true
    })?;
    info!(
        "Done Cleaning DB, deleted: {deleted}, took {}s",
        (Utc::now() - start).num_seconds()
    );

    print_db_stats(db_path.path());

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

async fn walk_snapshot<F, T>(tipset: &Tipset, mut load_block: F) -> anyhow::Result<CidHashSet>
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
                debug!(target: "chain_api", "export at: {}", current_min_height);
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

fn print_db_stats(path: &Path) {
    info!(
        "db_path: {}, size: {}",
        path.display(),
        fs_extra::dir::get_size(path)
            .unwrap_or_default()
            .human_count_bytes()
    );
}

struct MemStatsTracker {
    physical_mem: Arc<AtomicUsize>,
    virtual_mem: Arc<AtomicUsize>,
    cancelled: Arc<AtomicBool>,
}

impl MemStatsTracker {
    fn run_async(&self) {
        let physical_mem = self.physical_mem.clone();
        let virtual_mem = self.virtual_mem.clone();
        let cancelled = self.cancelled.clone();
        tokio::spawn(async move {
            while !cancelled.load(atomic::Ordering::Relaxed) {
                if let Some(usage) = memory_stats() {
                    physical_mem.fetch_max(usage.physical_mem, atomic::Ordering::Relaxed);
                    virtual_mem.fetch_max(usage.virtual_mem, atomic::Ordering::Relaxed);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
    }
}

impl Default for MemStatsTracker {
    fn default() -> Self {
        Self {
            physical_mem: Arc::new(AtomicUsize::new(0)),
            virtual_mem: Arc::new(AtomicUsize::new(0)),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Drop for MemStatsTracker {
    fn drop(&mut self) {
        self.cancelled.store(true, atomic::Ordering::Relaxed);
        info!(
            "Peak physical memory usage: {}",
            self.physical_mem
                .load(atomic::Ordering::Relaxed)
                .human_count_bytes()
        );
        info!(
            "Peak virtual memory usage: {}",
            self.virtual_mem
                .load(atomic::Ordering::Relaxed)
                .human_count_bytes()
        );
    }
}
