// Copyright 2019-2024 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use crate::rpc::{chain_api::ChainGetPath, types::*, RpcMethod};
use crate::{
    blocks::{CachingBlockHeader, Tipset, TipsetKey},
    rpc::chain_api::*,
    shim::clock::ChainEpoch,
};
use cid::Cid;

use super::{ApiInfo, JsonRpcError, RpcRequest};

impl ApiInfo {
    pub async fn chain_head(&self) -> Result<Tipset, JsonRpcError> {
        self.call(Self::chain_head_req()).await
    }

    pub fn chain_head_req() -> RpcRequest<Tipset> {
        RpcRequest::new(CHAIN_HEAD, ())
    }

    pub async fn chain_get_block(&self, cid: Cid) -> Result<CachingBlockHeader, JsonRpcError> {
        self.call(Self::chain_get_block_req(cid)).await
    }

    pub fn chain_get_block_req(cid: Cid) -> RpcRequest<CachingBlockHeader> {
        RpcRequest::new(CHAIN_GET_BLOCK, (cid,))
    }

    /// Get tipset at epoch. Pick younger tipset if epoch points to a
    /// null-tipset. Only tipsets below the given `head` are searched. If `head`
    /// is null, the node will use the heaviest tipset.
    pub async fn chain_get_tipset_by_height(
        &self,
        epoch: ChainEpoch,
        head: ApiTipsetKey,
    ) -> Result<Tipset, JsonRpcError> {
        self.call(Self::chain_get_tipset_by_height_req(epoch, head))
            .await
    }

    pub fn chain_get_tipset_by_height_req(
        epoch: ChainEpoch,
        head: ApiTipsetKey,
    ) -> RpcRequest<Tipset> {
        RpcRequest::new(CHAIN_GET_TIPSET_BY_HEIGHT, (epoch, head))
    }

    pub fn chain_get_tipset_after_height_req(
        epoch: ChainEpoch,
        head: ApiTipsetKey,
    ) -> RpcRequest<Tipset> {
        RpcRequest::new_v1(CHAIN_GET_TIPSET_AFTER_HEIGHT, (epoch, head))
    }

    #[allow(unused)] // consistency
    pub async fn chain_get_tipset(&self, tsk: TipsetKey) -> Result<Tipset, JsonRpcError> {
        self.call(Self::chain_get_tipset_req(tsk)).await
    }

    pub fn chain_get_tipset_req(tsk: TipsetKey) -> RpcRequest<Tipset> {
        RpcRequest::new(CHAIN_GET_TIPSET, (tsk,))
    }

    pub async fn chain_get_genesis(&self) -> Result<Option<Tipset>, JsonRpcError> {
        self.call(Self::chain_get_genesis_req()).await
    }

    pub fn chain_get_genesis_req() -> RpcRequest<Option<Tipset>> {
        RpcRequest::new(CHAIN_GET_GENESIS, ())
    }

    pub async fn chain_set_head(&self, new_head: TipsetKey) -> Result<(), JsonRpcError> {
        self.call(Self::chain_set_head_req(new_head)).await
    }

    pub fn chain_set_head_req(new_head: TipsetKey) -> RpcRequest<()> {
        RpcRequest::new(CHAIN_SET_HEAD, (new_head,))
    }

    pub fn chain_get_path_req(from: TipsetKey, to: TipsetKey) -> RpcRequest<Vec<PathChange>> {
        RpcRequest::new(ChainGetPath::NAME, (from, to))
    }

    pub async fn chain_get_min_base_fee(
        &self,
        basefee_lookback: u32,
    ) -> Result<String, JsonRpcError> {
        self.call(Self::chain_get_min_base_fee_req(basefee_lookback))
            .await
    }

    pub fn chain_get_min_base_fee_req(basefee_lookback: u32) -> RpcRequest<String> {
        RpcRequest::new(CHAIN_GET_MIN_BASE_FEE, (basefee_lookback,))
    }

    pub fn chain_notify_req() -> RpcRequest<()> {
        RpcRequest::new(CHAIN_NOTIFY, ())
    }
}
