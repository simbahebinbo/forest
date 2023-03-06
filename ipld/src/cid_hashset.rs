// Copyright 2019-2023 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use ahash::HashSet;
use cid::Cid;

#[derive(Default, Debug, Clone)]
pub struct CidHashSet(HashSet<u64>);

impl CidHashSet {
    pub fn insert(&mut self, cid: &Cid) -> bool {
        let hash = self.0.hasher().hash_one(cid);
        self.0.insert(hash)
    }

    pub fn contains(&self, cid: &Cid) -> bool {
        let hash = self.0.hasher().hash_one(cid);
        self.0.contains(&hash)
    }

    pub fn inner(&self) -> &HashSet<u64> {
        &self.0
    }

    pub fn inner_mut(&mut self) -> &mut HashSet<u64> {
        &mut self.0
    }
}
