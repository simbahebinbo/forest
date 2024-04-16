// Copyright 2019-2024 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use crate::rpc::{ApiVersion, Ctx, RpcMethod, ServerError};
use crate::{beacon::BeaconEntry, lotus_json::LotusJson, shim::clock::ChainEpoch};
use anyhow::Result;
use fvm_ipld_blockstore::Blockstore;

macro_rules! for_each_method {
    ($callback:ident) => {
        $callback!(crate::rpc::beacon::BeaconGetEntry);
    };
}
pub(crate) use for_each_method;

/// `BeaconGetEntry` returns the beacon entry for the given Filecoin epoch. If
/// the entry has not yet been produced, the call will block until the entry
/// becomes available
pub enum BeaconGetEntry {}
impl RpcMethod<1> for BeaconGetEntry {
    const NAME: &'static str = "Filecoin.BeaconGetEntry";
    const PARAM_NAMES: [&'static str; 1] = ["first"];
    const API_VERSION: ApiVersion = ApiVersion::V0;

    type Params = (ChainEpoch,);
    type Ok = LotusJson<BeaconEntry>;

    async fn handle(
        ctx: Ctx<impl Blockstore>,
        (first,): Self::Params,
    ) -> Result<Self::Ok, ServerError> {
        let (_, beacon) = ctx.beacon.beacon_for_epoch(first)?;
        let rr =
            beacon.max_beacon_round_for_epoch(ctx.state_manager.get_network_version(first), first);
        let e = beacon.entry(rr).await?;
        Ok(e.into())
    }
}