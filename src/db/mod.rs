// Copyright 2019-2023 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

mod errors;
mod memory;
mod metrics;

cfg_if::cfg_if! {
    if #[cfg(feature = "rocksdb")] {
        pub mod rocks;
    } else if #[cfg(feature = "paritydb")] {
        pub mod parity_db;
    }
}

// Not using conditional compilation here because DB config types are used in
// forest config
pub mod parity_db_config;
pub mod rocks_config;

pub use errors::Error;
pub use memory::MemoryDB;

#[cfg(any(feature = "paritydb", feature = "rocksdb"))]
pub mod rolling;

/// Store interface used as a KV store implementation
pub trait Store {
    /// Read single value from data store and return `None` if key doesn't
    /// exist.
    fn read<K>(&self, key: K) -> Result<Option<Vec<u8>>, Error>
    where
        K: AsRef<[u8]>;

    /// Write a single value to the data store.
    fn write<K, V>(&self, key: K, value: V) -> Result<(), Error>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>;

    /// Returns `Ok(true)` if key exists in store
    fn exists<K>(&self, key: K) -> Result<bool, Error>
    where
        K: AsRef<[u8]>;

    /// Write slice of KV pairs.
    fn bulk_write(
        &self,
        values: impl IntoIterator<Item = (impl Into<Vec<u8>>, impl Into<Vec<u8>>)>,
    ) -> Result<(), Error> {
        values
            .into_iter()
            .try_for_each(|(key, value)| self.write(key.into(), value.into()))
    }

    /// Flush writing buffer if there is any. Default implementation is blank
    fn flush(&self) -> Result<(), Error> {
        Ok(())
    }
}

impl<BS: Store> Store for &BS {
    fn read<K>(&self, key: K) -> Result<Option<Vec<u8>>, Error>
    where
        K: AsRef<[u8]>,
    {
        (*self).read(key)
    }

    fn write<K, V>(&self, key: K, value: V) -> Result<(), Error>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        (*self).write(key, value)
    }

    fn exists<K>(&self, key: K) -> Result<bool, Error>
    where
        K: AsRef<[u8]>,
    {
        (*self).exists(key)
    }

    fn bulk_write(
        &self,
        values: impl IntoIterator<Item = (impl Into<Vec<u8>>, impl Into<Vec<u8>>)>,
    ) -> Result<(), Error> {
        (*self).bulk_write(values)
    }
}

/// Traits for collecting DB stats
pub trait DBStatistics {
    fn get_statistics(&self) -> Option<String> {
        None
    }
}

#[cfg(any(feature = "paritydb", feature = "rocksdb"))]
pub mod db_engine {
    use std::path::{Path, PathBuf};

    use crate::db::rolling::*;

    cfg_if::cfg_if! {
        if #[cfg(feature = "rocksdb")] {
            pub type Db = crate::db::rocks::RocksDb;
            pub type DbConfig = crate::db::rocks_config::RocksDbConfig;
            pub(in crate::db) type DbError = rocksdb::Error;
            const DIR_NAME: &str = "rocksdb";
        } else if #[cfg(feature = "paritydb")] {
            pub type Db = crate::db::parity_db::ParityDb;
            pub type DbConfig = crate::db::parity_db_config::ParityDbConfig;
            pub(in crate::db) type DbError = parity_db::Error;
            const DIR_NAME: &str = "paritydb";
        }
    }

    pub fn db_root(chain_data_root: &Path) -> PathBuf {
        chain_data_root.join(DIR_NAME)
    }

    pub(in crate::db) fn open_db(path: &Path, config: &DbConfig) -> anyhow::Result<Db> {
        Db::open(path, config).map_err(Into::into)
    }

    pub fn open_proxy_db(db_root: PathBuf, db_config: DbConfig) -> anyhow::Result<RollingDB> {
        RollingDB::load_or_create(db_root, db_config)
    }
}
#[cfg(test)]
mod tests {
    pub mod db_utils;
    mod mem_test;
    #[cfg(feature = "paritydb")]
    mod parity_test;
    #[cfg(feature = "rocksdb")]
    mod rocks_test;
    pub mod subtests;
}