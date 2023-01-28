// Copyright 2019-2023 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use super::*;
use crate::DBStatistics;
use cid::Cid;
use forest_libp2p_bitswap::BitswapStore;
use fvm_ipld_blockstore::Blockstore;
use std::sync::Arc;

#[derive(Debug)]
struct ProxyStoreInner<T> {
    persistent: T,
    rolling: RollingStore<T>,
}

#[derive(Debug, Clone)]
pub struct ProxyStore<T>(Arc<ProxyStoreInner<T>>);

impl<T> ProxyStore<T> {
    pub fn new(persistent: T, rolling: RollingStore<T>) -> Self {
        Self(Arc::new(ProxyStoreInner {
            persistent,
            rolling,
        }))
    }

    pub fn persistent(&self) -> &T {
        &self.0.persistent
    }

    pub fn rolling(&self) -> &RollingStore<T> {
        &self.0.rolling
    }
}

impl<T> ReadStore for ProxyStore<T>
where
    T: ReadStore + IndexedStore + 'static,
{
    fn read<K>(&self, key: K) -> Result<Option<Vec<u8>>, crate::Error>
    where
        K: AsRef<[u8]>,
    {
        if let Some(data) = self.persistent().read(key.as_ref())? {
            Ok(Some(data))
        } else {
            self.rolling().read(key)
        }
    }

    fn exists<K>(&self, key: K) -> Result<bool, crate::Error>
    where
        K: AsRef<[u8]>,
    {
        Ok(self.persistent().exists(key.as_ref())? || self.rolling().exists(key)?)
    }

    // TODO: Merge results from persistent and rolling, use fallback implementation for now
    // fn bulk_read<K>(&self, keys: &[K]) -> Result<Vec<Option<Vec<u8>>>, crate::Error>
    // where
    //     K: AsRef<[u8]>,
    // {
    //
    // }
}

impl<T> DBStatistics for ProxyStore<T>
where
    T: DBStatistics + 'static,
{
    fn get_statistics(&self) -> Option<String> {
        self.persistent().get_statistics()
    }
}

// FIXME: It should not be the persistent one, maybe use the latest rolling one
impl<T> BitswapStore for ProxyStore<T>
where
    T: BitswapStore,
{
    type Params = <T as BitswapStore>::Params;

    fn contains(&self, cid: &Cid) -> anyhow::Result<bool> {
        self.persistent().contains(cid)
    }

    fn get(&self, cid: &Cid) -> anyhow::Result<Option<Vec<u8>>> {
        self.persistent().get(cid)
    }

    fn insert(&self, block: &libipld::Block<Self::Params>) -> anyhow::Result<()> {
        self.persistent().insert(block)
    }
}

impl<T> Blockstore for ProxyStore<T>
where
    T: ReadStore + IndexedStore + 'static,
{
    fn get(&self, k: &Cid) -> anyhow::Result<Option<Vec<u8>>> {
        self.read(k.to_bytes()).map_err(|e| e.into())
    }

    fn put_keyed(&self, _k: &Cid, _block: &[u8]) -> anyhow::Result<()> {
        unimplemented!("Use an inner writable store instead")
        // self.write(_k.to_bytes(), _block).map_err(|e| e.into())
    }
}

impl<T> ReadWriteStore for ProxyStore<T>
where
    T: ReadWriteStore + IndexedStore + 'static,
{
    fn write<K, V>(&self, _key: K, _value: V) -> Result<(), crate::Error>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        unimplemented!()
        // self.persistent().write(_key, _value)
    }

    fn delete<K>(&self, _key: K) -> Result<(), crate::Error>
    where
        K: AsRef<[u8]>,
    {
        unimplemented!()
        // self.persistent().delete(_key)
    }
}

impl Store for ProxyStore<crate::db_engine::Db> {
    fn persistent(&self) -> &crate::db_engine::Db {
        self.persistent()
    }

    fn rolling_by_epoch(&self, epoch: i64) -> SplitStore<Self, crate::db_engine::Db> {
        const EPOCHS_IN_DAY: i64 = 2880;
        let index = epoch / EPOCHS_IN_DAY;
        SplitStore {
            r: self.clone(),
            w: self.rolling().get_writable_store(index as _).unwrap(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SplitStore<R, W>
where
    R: ReadStore,
    W: ReadWriteStore,
{
    r: R,
    w: W,
}

impl<R, W> Blockstore for SplitStore<R, W>
where
    R: ReadStore,
    W: ReadWriteStore,
{
    fn get(&self, k: &Cid) -> anyhow::Result<Option<Vec<u8>>> {
        self.r.read(k.to_bytes()).map_err(|e| e.into())
    }

    fn put_keyed(&self, k: &Cid, block: &[u8]) -> anyhow::Result<()> {
        self.w.write(k.to_bytes(), block).map_err(|e| e.into())
    }

    // FIXME
    // fn put_many_keyed<D, I>(&self, blocks: I) -> anyhow::Result<()>
    // where
    //     Self: Sized,
    //     D: AsRef<[u8]>,
    //     I: IntoIterator<Item = (Cid, D)>,
    // {
    //     todo!()
    // }
}

impl<R, W> ReadStore for SplitStore<R, W>
where
    R: ReadStore,
    W: ReadWriteStore,
{
    fn read<K>(&self, key: K) -> Result<Option<Vec<u8>>, crate::Error>
    where
        K: AsRef<[u8]>,
    {
        self.r.read(key)
    }

    fn exists<K>(&self, key: K) -> Result<bool, crate::Error>
    where
        K: AsRef<[u8]>,
    {
        self.r.exists(key)
    }
}

impl<R, W> ReadWriteStore for SplitStore<R, W>
where
    R: ReadStore,
    W: ReadWriteStore,
{
    fn write<K, V>(&self, key: K, value: V) -> Result<(), crate::Error>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        self.w.write(key, value)
    }

    fn delete<K>(&self, key: K) -> Result<(), crate::Error>
    where
        K: AsRef<[u8]>,
    {
        self.w.delete(key)
    }
}
