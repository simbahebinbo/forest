// Copyright 2019-2023 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::{
    collections::VecDeque,
    sync::{
        atomic::{self, AtomicUsize},
        Arc,
    },
    time,
};

use anyhow::bail;
use cid::{multihash::Code, Cid};
use forest_blocks::{BlockHeader, Tipset, TipsetKeys};
use forest_db::{ReadWriteStore, Store};
use forest_ipld::{recurse_links_hash, CidHashSet};
use forest_state_manager::StateManager;
use forest_utils::{db::BlockstoreExt, net::FetchProgress};
use futures::Future;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_car::{load_car, CarReader};
use fvm_ipld_encoding::Cbor;
use log::{debug, info};
use tokio::{
    fs::File,
    io::{AsyncRead, BufReader},
};
use tokio_util::compat::TokioAsyncReadCompatExt;
use url::Url;

#[cfg(feature = "testing")]
pub const EXPORT_SR_40: &[u8] = std::include_bytes!("export40.car");

/// Uses an optional file path or the default genesis to parse the genesis and
/// determine if chain store has existing data for the given genesis.
pub async fn read_genesis_header<DB>(
    genesis_fp: Option<&String>,
    genesis_bytes: Option<&[u8]>,
    db: &DB,
) -> Result<BlockHeader, anyhow::Error>
where
    DB: Blockstore + Store + Send + Sync,
{
    let genesis = match genesis_fp {
        Some(path) => {
            let file = File::open(path).await?;
            let reader = BufReader::new(file);
            process_car(reader, db).await?
        }
        None => {
            debug!("No specified genesis in config. Using default genesis.");
            let genesis_bytes =
                genesis_bytes.ok_or_else(|| anyhow::anyhow!("No default genesis."))?;
            let reader = BufReader::<&[u8]>::new(genesis_bytes);
            process_car(reader, db).await?
        }
    };

    info!("Initialized genesis: {}", genesis);
    Ok(genesis)
}

pub fn get_network_name_from_genesis<BS>(
    genesis_header: &BlockHeader,
    state_manager: &StateManager<BS>,
) -> Result<String, anyhow::Error>
where
    BS: Blockstore + Store + Clone + Send + Sync + 'static,
{
    // Get network name from genesis state.
    let network_name = state_manager
        .get_network_name(genesis_header.state_root())
        .map_err(|e| anyhow::anyhow!("Failed to retrieve network name from genesis: {}", e))?;
    Ok(network_name)
}

pub async fn initialize_genesis<BS>(
    genesis_fp: Option<&String>,
    state_manager: &StateManager<BS>,
) -> Result<(Tipset, String), anyhow::Error>
where
    BS: Blockstore + Store + Clone + Send + Sync + 'static,
{
    let genesis_bytes = state_manager.chain_config().genesis_bytes();
    let genesis =
        read_genesis_header(genesis_fp, genesis_bytes, state_manager.blockstore()).await?;
    let ts = Tipset::from(&genesis);
    let network_name = get_network_name_from_genesis(&genesis, state_manager)?;
    Ok((ts, network_name))
}

async fn process_car<R, BS>(reader: R, db: &BS) -> Result<BlockHeader, anyhow::Error>
where
    R: AsyncRead + Send + Unpin,
    BS: Store + Send + Sync,
{
    let db = db.persistent();

    // Load genesis state into the database and get the Cid
    let genesis_cids: Vec<Cid> = load_car(db, reader.compat()).await?;
    if genesis_cids.len() != 1 {
        panic!("Invalid Genesis. Genesis Tipset must have only 1 Block.");
    }

    let genesis_block: BlockHeader = db.get_obj(&genesis_cids[0])?.ok_or_else(|| {
        anyhow::anyhow!("Could not find genesis block despite being loaded using a genesis file")
    })?;

    Ok(genesis_block)
}

/// Import a chain from a CAR file. If the snapshot boolean is set, it will not
/// verify the chain state and instead accept the largest height as genesis.
pub async fn import_chain<DB>(
    sm: &Arc<StateManager<DB>>,
    path: &str,
    validate_height: Option<i64>,
    skip_load: bool,
) -> Result<(), anyhow::Error>
where
    DB: Blockstore + Store + Clone + Send + Sync + 'static,
{
    let is_remote_file: bool = path.starts_with("http://") || path.starts_with("https://");

    info!("Importing chain from snapshot at: {path}");
    // start import
    let stopwatch = time::Instant::now();
    let cids = if is_remote_file {
        info!("Downloading file...");
        let url = Url::parse(path)?;
        let reader = FetchProgress::fetch_from_url(url).await?;
        load_and_retrieve_header(sm.blockstore(), reader, skip_load).await?
    } else {
        info!("Reading file...");
        let file = File::open(&path).await?;
        let reader = FetchProgress::fetch_from_file(file).await?;
        load_and_retrieve_header(sm.blockstore(), reader, skip_load).await?
    };

    let ts = sm.chain_store().tipset_from_keys(&TipsetKeys::new(cids))?;

    let n_cids = Arc::new(AtomicUsize::new(0));
    walk_snapshot(&ts, |cid| {
        let db0 = sm.blockstore().rolling_by_epoch_raw(0).store;
        let db_base = sm.blockstore().persistent();
        let n_cids = n_cids.clone();
        async move {
            let block = Blockstore::get(&db0, &cid)?
                .ok_or_else(|| anyhow::anyhow!("Cid {cid} not found in blockstore"))?;

            // Don't include identity CIDs.
            // We only include raw and dagcbor, for now.
            // Raw for "code" CIDs.
            if u64::from(Code::Identity) != cid.hash().code()
                && (cid.codec() == fvm_shared::IPLD_RAW
                    || cid.codec() == fvm_ipld_encoding::DAG_CBOR)
            {
                n_cids.fetch_add(1, atomic::Ordering::Relaxed);
                db_base.put_keyed(&cid, block.as_slice())?;
                db0.delete(cid.to_bytes())?;
            }

            Ok(block)
        }
    })
    .await?;

    info!(
        "{} CIDs written to persistent DB",
        n_cids.load(atomic::Ordering::Relaxed)
    );
    info!("Loaded .car file in {}s", stopwatch.elapsed().as_secs());

    if !skip_load {
        let gb = sm.chain_store().tipset_by_height(0, ts.clone(), true)?;
        sm.chain_store().set_genesis(&gb.blocks()[0])?;
        if !matches!(&sm.chain_config().genesis_cid, Some(expected_cid) if expected_cid ==  &gb.blocks()[0].cid().to_string())
        {
            bail!(
                "Snapshot incompatible with {}. Consider specifying the network with `--chain` flag or 
                 use a custom config file to set expected genesis CID for selected network", 
                sm.chain_config().name
            );
        }
    }

    // Update head with snapshot header tipset
    sm.chain_store().set_heaviest_tipset(ts.clone())?;
    sm.blockstore().flush()?;

    if let Some(height) = validate_height {
        let height = if height > 0 {
            height
        } else {
            (ts.epoch() + height).max(0)
        };
        info!("Validating imported chain from height: {}", height);
        sm.validate_chain(ts.clone(), height).await?;
    }

    info!("Accepting {:?} as new head.", ts.cids());

    Ok(())
}

/// Loads car file into database, and returns the block header CIDs from the CAR
/// header.
async fn load_and_retrieve_header<DB, R>(
    store: &DB,
    reader: FetchProgress<R>,
    skip_load: bool,
) -> anyhow::Result<Vec<Cid>>
where
    DB: Store,
    R: AsyncRead + Send + Unpin,
{
    let mut compat = reader.compat();
    let result = if skip_load {
        CarReader::new(&mut compat).await?.header.roots
    } else {
        forest_load_car(store.rolling_by_epoch_raw(0).store, &mut compat).await?
    };
    compat.into_inner().finish();

    Ok(result)
}

/// Optimizations:
/// 1. ParityDB could benefit from a larger buffer. It's hard coded as 1000
/// blocks in [fvm_ipld_car::load_car] 2. Use [Store::bulk_write] instead of
/// [Blockstore] to avoid tons of unneccesary allocations
pub async fn forest_load_car<DB, R>(store: DB, reader: R) -> anyhow::Result<Vec<Cid>>
where
    R: futures::AsyncRead + Send + Unpin,
    DB: ReadWriteStore,
{
    // 1GB
    const BUFFER_CAPCITY_BYTES: usize = 1024 * 1024 * 1024;

    let mut n_cids = 0;
    let mut car_reader = CarReader::new(reader).await?;
    let mut estimated_size = 0;
    let mut buffer = vec![];
    while let Some(block) = car_reader.next_block().await? {
        n_cids += 1;
        estimated_size += 64 + block.data.len();
        buffer.push((block.cid.to_bytes(), block.data));
        if estimated_size >= BUFFER_CAPCITY_BYTES {
            store.bulk_write(std::mem::take(&mut buffer))?;
            estimated_size = 0;
        }
    }
    store.bulk_write(buffer)?;
    info!("{n_cids} CIDs loaded from snapshot");
    Ok(car_reader.header.roots)
}

pub async fn walk_snapshot<F, T>(tipset: &Tipset, mut load_block: F) -> anyhow::Result<()>
where
    F: FnMut(Cid) -> T + Send,
    T: Future<Output = anyhow::Result<Vec<u8>>> + Send,
{
    let mut seen = CidHashSet::default();
    let mut blocks_to_walk: VecDeque<Cid> = tipset.cids().to_vec().into();
    let mut current_min_height = tipset.epoch();

    while let Some(next) = blocks_to_walk.pop_front() {
        if !seen.insert(&next) {
            continue;
        }

        let data = load_block(next).await?;

        let h = BlockHeader::unmarshal_cbor(&data)?;

        if current_min_height > h.epoch() {
            current_min_height = h.epoch();
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

        if h.epoch() == 0 {
            recurse_links_hash(&mut seen, *h.state_root(), &mut load_block).await?;
        }
    }

    Ok(())
}
