// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use crate::{tipset_from_keys, Error, TipsetCache};
use async_std::task;
use forest_blocks::{Tipset, TipsetKeys};
use forest_utils::io::ProgressBar;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::clock::ChainEpoch;
use lru::LruCache;
use std::{num::NonZeroUsize, sync::Arc};
use tokio::sync::RwLock;

const DEFAULT_CHAIN_INDEX_CACHE_SIZE: NonZeroUsize =
    forest_utils::const_option!(NonZeroUsize::new(32 << 10));

/// Configuration which sets the length of tipsets to skip in between each cached entry.
const SKIP_LENGTH: ChainEpoch = 20;

// this module helps speed up boot times for forest by checkpointing previously seen tipsets from snapshots.
pub(super) mod checkpoint_tipsets {
    use cid::Cid;
    use forest_blocks::TipsetKeys;
    use once_cell::sync::Lazy;
    use std::collections::HashMap;
    use std::str::FromStr;

    macro_rules! add_calibnet {
        ($map: ident, $key_hash:expr) => {
            $map.insert(
                $key_hash,
                // calibnet genesis tipset keys
                TipsetKeys::new(vec![Cid::from_str(
                    "bafy2bzacecyaggy24wol5ruvs6qm73gjibs2l2iyhcqmvi7r7a4ph7zx3yqd4",
                )
                .unwrap()]),
            );
        };
    }

    macro_rules! add_mainnet {
        ($map: ident, $key_hash:expr) => {
            $map.insert(
                $key_hash,
                // mainnet genesis tipset keys
                TipsetKeys::new(vec![Cid::from_str(
                    "bafy2bzacecnamqgqmifpluoeldx7zzglxcljo6oja4vrmtj7432rphldpdmm2",
                )
                .unwrap()]),
            );
        };
    }

    // The hashes for these checkpoints is obtained by passing the tipset keys' cids (tipset.cids())
    // through a blake2b hasher.
    type TipsetKeyHash = &'static str;
    type GenesisTipsetCids = TipsetKeys;
    // A map of validated checkpoints mapping to their genesis tipkeys
    type TipsetCheckpointsRegistry = HashMap<TipsetKeyHash, GenesisTipsetCids>;
    // Represents a static map of validated tipset hashes which helps to remove the need to validate the tipset
    // back to genesis if it has been validated before, thereby reducing boot times.
    pub(super) static TIPSET_CHECKPOINTS: Lazy<TipsetCheckpointsRegistry> = Lazy::new(|| {
        let mut map = HashMap::new();
        // The numeric suffix in the variable name is the height of the snapshot.
        // NB: Add desired tipset checkpoints below this by using RPC command: forest-cli chain tipset-hash <cid keys>
        const CALIBNET_CHECKPOINT_1405400: TipsetKeyHash = "7930ad8bf32b35314b3bc47b9e25249af8ec6ba7f5544c05e8b5bd3b3ec09f76df8bd2278f9b318badf1a08d0a468abd55130465c6c55f99e67badc0e614ca79";
        const MAINNET_CHECKPOINT_2325300: TipsetKeyHash = "319f2351ceaf78fbcc8688dc75a19bdf8ee6e895e547ff5cc2f7b18a3a36b65ff94c1860733137d0244352f82ba6fd9672aec14deee358e7cf6e088bf89a28b1";
        add_calibnet!(map, CALIBNET_CHECKPOINT_1405400);
        add_mainnet!(map, MAINNET_CHECKPOINT_2325300);
        map
    });

    pub(super) fn genesis_from_checkpoint_tipset(tsk: &TipsetKeys) -> Option<GenesisTipsetCids> {
        TIPSET_CHECKPOINTS.get(&tipset_hash(tsk).as_str()).cloned()
    }

    pub fn tipset_hash(tsk: &TipsetKeys) -> String {
        let ts_bytes: Vec<_> = tsk.cids().iter().flat_map(|s| s.to_bytes()).collect();
        let tipset_keys_hash = blake2b_simd::blake2b(&ts_bytes).to_hex();
        tipset_keys_hash.to_string()
    }
}

/// `Lookback` entry to cache in the `ChainIndex`. Stores all relevant info when doing `lookbacks`.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct LookbackEntry {
    tipset: Arc<Tipset>,
    parent_height: ChainEpoch,
    target_height: ChainEpoch,
    target: TipsetKeys,
}

/// Keeps look-back tipsets in cache at a given interval `skip_length` and can be used to look-back
/// at the chain to retrieve an old tipset.
pub(crate) struct ChainIndex<BS> {
    /// Cache of look-back entries to speed up lookup.
    skip_cache: RwLock<LruCache<TipsetKeys, Arc<LookbackEntry>>>,

    /// `Arc` reference tipset cache.
    ts_cache: Arc<TipsetCache>,

    /// `Blockstore` pointer needed to load tipsets from cold storage.
    db: BS,
}

impl<BS: Blockstore> ChainIndex<BS> {
    pub(crate) fn new(ts_cache: Arc<TipsetCache>, db: BS) -> Self {
        Self {
            skip_cache: RwLock::new(LruCache::new(DEFAULT_CHAIN_INDEX_CACHE_SIZE)),
            ts_cache,
            db,
        }
    }

    async fn load_tipset(&self, tsk: &TipsetKeys) -> Result<Arc<Tipset>, Error> {
        tipset_from_keys(self.ts_cache.as_ref(), &self.db, tsk).await
    }

    /// Loads tipset at `to` [`ChainEpoch`], loading from sparse cache and/or loading parents
    /// from the `blockstore`.
    pub(crate) async fn get_tipset_by_height(
        &self,
        from: Arc<Tipset>,
        to: ChainEpoch,
    ) -> Result<Arc<Tipset>, Error> {
        if from.epoch() - to <= SKIP_LENGTH {
            return self.walk_back(from, to).await;
        }
        let total_size = from.epoch() - to;
        let pb = ProgressBar::new(total_size as u64);
        pb.message("Scanning blockchain ");
        pb.set_max_refresh_rate(Some(std::time::Duration::from_millis(500)));

        let rounded = self.round_down(from).await?;

        let mut cur = rounded.key().clone();
        const MAX_COUNT: usize = 100;
        let mut counter = 0;
        loop {
            let entry = self.skip_cache.write().await.get(&cur).cloned();
            let lbe = if let Some(cached) = entry {
                cached
            } else {
                self.fill_cache(std::mem::take(&mut cur)).await?
            };

            if let Some(genesis_tipset_keys) =
                checkpoint_tipsets::genesis_from_checkpoint_tipset(lbe.tipset.key())
            {
                let tipset =
                    tipset_from_keys(&self.ts_cache, &self.db, &genesis_tipset_keys).await?;
                return Ok(tipset);
            }

            if lbe.tipset.epoch() == to || lbe.parent_height < to {
                return Ok(lbe.tipset.clone());
            } else if to > lbe.target_height {
                return self.walk_back(lbe.tipset.clone(), to).await;
            }
            let to_be_done = lbe.tipset.epoch() - to;
            // Don't show the progress bar if we're doing less than 10_000 units of work.
            if total_size > 10_000 {
                pb.set((total_size - to_be_done) as u64);
            }

            cur = lbe.target.clone();

            if counter == MAX_COUNT {
                counter = 0;
                task::yield_now().await;
            } else {
                counter += 1;
            }
        }
    }

    /// Walks back from the tipset, ignoring the cached entries.
    /// This should only be used when the cache is checked to be invalidated.
    pub(crate) async fn get_tipset_by_height_without_cache(
        &self,
        from: Arc<Tipset>,
        to: ChainEpoch,
    ) -> Result<Arc<Tipset>, Error> {
        self.walk_back(from, to).await
    }

    /// Fills cache with look-back entry, and returns inserted entry.
    async fn fill_cache(&self, tsk: TipsetKeys) -> Result<Arc<LookbackEntry>, Error> {
        let tipset = self.load_tipset(&tsk).await?;

        if tipset.epoch() == 0 {
            return Ok(Arc::new(LookbackEntry {
                tipset,
                parent_height: 0,
                target_height: Default::default(),
                target: Default::default(),
            }));
        }

        let parent = self.load_tipset(tipset.parents()).await?;
        let r_height = self.round_height(tipset.epoch()) - SKIP_LENGTH;

        let parent_epoch = parent.epoch();
        let skip_target = if parent.epoch() < r_height {
            parent
        } else {
            self.walk_back(parent, r_height).await?
        };

        let lbe = Arc::new(LookbackEntry {
            tipset,
            parent_height: parent_epoch,
            target_height: skip_target.epoch(),
            target: skip_target.key().clone(),
        });

        self.skip_cache.write().await.put(tsk.clone(), lbe.clone());
        Ok(lbe)
    }

    /// Rounds height epoch to nearest sparse cache index epoch.
    fn round_height(&self, height: ChainEpoch) -> ChainEpoch {
        (height / SKIP_LENGTH) * SKIP_LENGTH
    }

    /// Gets the closest rounded sparse index and returns the loaded tipset at that index.
    async fn round_down(&self, ts: Arc<Tipset>) -> Result<Arc<Tipset>, Error> {
        let target = self.round_height(ts.epoch());

        self.walk_back(ts, target).await
    }

    /// Load parent tipsets until the `to` [`ChainEpoch`].
    async fn walk_back(&self, from: Arc<Tipset>, to: ChainEpoch) -> Result<Arc<Tipset>, Error> {
        if to > from.epoch() {
            return Err(Error::Other(
                "Looking for tipset with height greater than start point".to_string(),
            ));
        }

        if to == from.epoch() {
            return Ok(from);
        }

        let mut ts = from;
        loop {
            let pts = self.load_tipset(ts.parents()).await?;

            if to > pts.epoch() {
                // Pts is lower than to epoch, return the tipset above that height
                return Ok(ts);
            }

            if to == pts.epoch() {
                return Ok(pts);
            }
            ts = pts;
        }
    }
}
