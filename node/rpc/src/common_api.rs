// Copyright 2019-2023 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT
#![allow(clippy::unused_async)]

use forest_beacon::Beacon;
use forest_db::Store;
use forest_rpc_api::{
    common_api::*,
    data_types::{APIVersion, RPCState, Version},
};
use fvm_ipld_blockstore::Blockstore;
use jsonrpc_v2::{Data, Error as JsonRpcError};
use semver::Version as SemVer;
use tokio::sync::mpsc::Sender;

pub(crate) async fn version(
    block_delay: u64,
    forest_version: &'static str,
) -> Result<VersionResult, JsonRpcError> {
    let v = SemVer::parse(forest_version).unwrap();
    Ok(APIVersion {
        version: forest_version.to_string(),
        api_version: Version::new(v.major, v.minor, v.patch),
        block_delay,
    })
}

pub(crate) async fn shutdown(shutdown_send: Sender<()>) -> Result<ShutdownResult, JsonRpcError> {
    // Trigger graceful shutdown
    if let Err(err) = shutdown_send.send(()).await {
        return Err(JsonRpcError::from(err));
    }
    Ok(())
}

/// gets start time from network
pub(crate) async fn start_time<
    DB: Blockstore + Store + Clone + Send + Sync + 'static,
    B: Beacon,
>(
    data: Data<RPCState<DB, B>>,
) -> Result<StartTimeResult, JsonRpcError> {
    Ok(data.start_time)
}
