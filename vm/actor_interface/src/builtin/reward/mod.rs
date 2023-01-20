// Copyright 2019-2023 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use forest_utils::db::BlockstoreExt;
use fvm::state_tree::ActorState;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use serde::Serialize;

use anyhow::Context;

/// Reward actor address.
pub const ADDRESS: Address = Address::new_id(2);

/// Reward actor method.
pub type Method = fil_actor_reward_v8::Method;

pub fn is_v8_reward_cid(cid: &Cid) -> bool {
    let known_cids = vec![
        // calibnet v8
        Cid::try_from("bafk2bzaceayah37uvj7brl5no4gmvmqbmtndh5raywuts7h6tqbgbq2ge7dhu").unwrap(),
        // mainnet
        Cid::try_from("bafk2bzacecwzzxlgjiavnc3545cqqil3cmq4hgpvfp2crguxy2pl5ybusfsbe").unwrap(),
        // devnet
        Cid::try_from("bafk2bzacedn3fkp27ys5dxn4pwqdq2atj2x6cyezxuekdorvjwi7zazirgvgy").unwrap(),
    ];
    known_cids.contains(cid)
}

pub fn is_v9_reward_cid(cid: &Cid) -> bool {
    let known_cids = vec![
        // calibnet v9
        Cid::try_from("bafk2bzacebpptqhcw6mcwdj576dgpryapdd2zfexxvqzlh3aoc24mabwgmcss").unwrap(),
        // mainnet v9
        Cid::try_from("bafk2bzacebezgbbmcm2gbcqwisus5fjvpj7hhmu5ubd37phuku3hmkfulxm2o").unwrap(),
    ];
    known_cids.contains(cid)
}

/// Reward actor state.
#[derive(Serialize)]
#[serde(untagged)]
pub enum State {
    V8(fil_actor_reward_v8::State),
    V9(fil_actor_reward_v9::State),
}

impl State {
    pub fn load<BS>(store: &BS, actor: &ActorState) -> anyhow::Result<State>
    where
        BS: Blockstore,
    {
        if is_v8_reward_cid(&actor.code) {
            return store
                .get_obj(&actor.state)?
                .map(State::V8)
                .context("Actor state doesn't exist in store");
        }
        if is_v9_reward_cid(&actor.code) {
            return store
                .get_obj(&actor.state)?
                .map(State::V8)
                .context("Actor state doesn't exist in store");
        }
        Err(anyhow::anyhow!("Unknown reward actor code {}", actor.code))
    }

    /// Consume state to return just storage power reward
    pub fn into_total_storage_power_reward(self) -> TokenAmount {
        match self {
            State::V8(st) => st.into_total_storage_power_reward(),
            State::V9(st) => st.into_total_storage_power_reward(),
        }
    }
}
