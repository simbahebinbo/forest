// Copyright 2019-2024 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT
#![allow(clippy::unused_async)]

use crate::blocks::TipsetKey;
use crate::chain::{BASE_FEE_MAX_CHANGE_DENOM, BLOCK_GAS_TARGET};
use crate::lotus_json::LotusJson;
use crate::message::{ChainMessage, Message as MessageTrait, SignedMessage};
use crate::rpc::{error::ServerError, types::*, ApiVersion, Ctx, RpcMethod};
use crate::shim::{
    address::{Address, Protocol},
    crypto::{Signature, SignatureType, SECP_SIG_LEN},
    econ::{TokenAmount, BLOCK_GAS_LIMIT},
    message::Message,
};
use fvm_ipld_blockstore::Blockstore;
use jsonrpsee::types::Params;
use num::BigInt;
use num_traits::{FromPrimitive, Zero};
use rand_distr::{Distribution, Normal};

use anyhow::{Context, Result};

const MIN_GAS_PREMIUM: f64 = 100000.0;

pub const GAS_ESTIMATE_FEE_CAP: &str = "Filecoin.GasEstimateFeeCap";
pub const GAS_ESTIMATE_GAS_PREMIUM: &str = "Filecoin.GasEstimateGasPremium";
pub const GAS_ESTIMATE_GAS_LIMIT: &str = "Filecoin.GasEstimateGasLimit";
pub const GAS_ESTIMATE_MESSAGE_GAS: &str = "Filecoin.GasEstimateMessageGas";

macro_rules! for_each_method {
    ($callback:ident) => {
        $callback!(crate::rpc::gas::GasEstimateGasLimit);
    };
}
pub(crate) use for_each_method;

/// Estimate the fee cap
pub async fn gas_estimate_fee_cap<DB: Blockstore>(
    params: Params<'_>,
    data: Ctx<DB>,
) -> Result<String, ServerError> {
    let LotusJson((msg, max_queue_blks, tsk)): LotusJson<(Message, i64, ApiTipsetKey)> =
        params.parse()?;

    estimate_fee_cap::<DB>(&data, msg, max_queue_blks, tsk).map(|n| TokenAmount::to_string(&n))
}

fn estimate_fee_cap<DB: Blockstore>(
    data: &Ctx<DB>,
    msg: Message,
    max_queue_blks: i64,
    _: ApiTipsetKey,
) -> Result<TokenAmount, ServerError> {
    let ts = data.state_manager.chain_store().heaviest_tipset();

    let parent_base_fee = &ts.block_headers().first().parent_base_fee;
    let increase_factor =
        (1.0 + (BASE_FEE_MAX_CHANGE_DENOM as f64).recip()).powf(max_queue_blks as f64);

    let fee_in_future = parent_base_fee
        * BigInt::from_f64(increase_factor * (1 << 8) as f64)
            .context("failed to convert fee_in_future f64 to bigint")?;
    let mut out: crate::shim::econ::TokenAmount = fee_in_future.div_floor(1 << 8);
    out += msg.gas_premium();
    Ok(out)
}

/// Estimate the fee cap
pub async fn gas_estimate_gas_premium<DB: Blockstore>(
    params: Params<'_>,
    data: Ctx<DB>,
) -> Result<String, ServerError> {
    let LotusJson((nblocksincl, _sender, _gas_limit, _)): LotusJson<(
        u64,
        Address,
        i64,
        TipsetKey,
    )> = params.parse()?;

    estimate_gas_premium::<DB>(&data, nblocksincl)
        .await
        .map(|n| TokenAmount::to_string(&n))
}

pub async fn estimate_gas_premium<DB: Blockstore>(
    data: &Ctx<DB>,
    mut nblocksincl: u64,
) -> Result<TokenAmount, ServerError> {
    if nblocksincl == 0 {
        nblocksincl = 1;
    }

    struct GasMeta {
        pub price: TokenAmount,
        pub limit: u64,
    }

    let mut prices: Vec<GasMeta> = Vec::new();
    let mut blocks = 0;

    let mut ts = data.state_manager.chain_store().heaviest_tipset();

    for _ in 0..(nblocksincl * 2) {
        if ts.epoch() == 0 {
            break;
        }
        let pts = data
            .state_manager
            .chain_store()
            .chain_index
            .load_required_tipset(ts.parents())?;
        blocks += pts.block_headers().len();
        let msgs = crate::chain::messages_for_tipset(data.state_manager.blockstore_owned(), &pts)?;

        prices.append(
            &mut msgs
                .iter()
                .map(|msg| GasMeta {
                    price: msg.message().gas_premium(),
                    limit: msg.message().gas_limit(),
                })
                .collect(),
        );
        ts = pts;
    }

    prices.sort_by(|a, b| b.price.cmp(&a.price));
    let mut at = BLOCK_GAS_TARGET * blocks as u64 / 2;
    let mut prev = TokenAmount::zero();
    let mut premium = TokenAmount::zero();

    for price in prices {
        at -= price.limit;
        if at > 0 {
            prev = price.price;
            continue;
        }
        if prev == TokenAmount::zero() {
            let ret: TokenAmount = price.price + TokenAmount::from_atto(1);
            return Ok(ret);
        }
        premium = (&price.price + &prev).div_floor(2) + TokenAmount::from_atto(1)
    }

    if premium == TokenAmount::zero() {
        premium = TokenAmount::from_atto(match nblocksincl {
            1 => (MIN_GAS_PREMIUM * 2.0) as u64,
            2 => (MIN_GAS_PREMIUM * 1.5) as u64,
            _ => MIN_GAS_PREMIUM as u64,
        });
    }

    let precision = 32;

    // mean 1, stddev 0.005 => 95% within +-1%
    let noise: f64 = Normal::new(1.0, 0.005)
        .unwrap()
        .sample(&mut rand::thread_rng());

    premium *= BigInt::from_f64(noise * (1i64 << precision) as f64)
        .context("failed to convert gas premium f64 to bigint")?;
    premium = premium.div_floor(1i64 << precision);

    Ok(premium)
}

pub enum GasEstimateGasLimit {}
impl RpcMethod<2> for GasEstimateGasLimit {
    const NAME: &'static str = "Filecoin.GasEstimateGasLimit";
    const PARAM_NAMES: [&'static str; 2] = ["msg", "tsk"];
    const API_VERSION: ApiVersion = ApiVersion::V0;

    type Params = (LotusJson<Message>, LotusJson<ApiTipsetKey>);
    type Ok = i64;

    async fn handle(
        ctx: Ctx<impl Blockstore + Send + Sync + 'static>,
        (LotusJson(msg), LotusJson(tsk)): Self::Params,
    ) -> Result<Self::Ok, ServerError> {
        estimate_gas_limit(&ctx, msg, tsk).await
    }
}

async fn estimate_gas_limit<DB>(
    data: &Ctx<DB>,
    msg: Message,
    ApiTipsetKey(tsk): ApiTipsetKey,
) -> Result<i64, ServerError>
where
    DB: Blockstore + Send + Sync + 'static,
{
    let mut msg = msg;
    msg.set_gas_limit(BLOCK_GAS_LIMIT);
    msg.set_gas_fee_cap(TokenAmount::from_atto(0));
    msg.set_gas_premium(TokenAmount::from_atto(0));

    let curr_ts = data
        .state_manager
        .chain_store()
        .load_required_tipset_or_heaviest(&tsk)?;
    let from_a = data
        .state_manager
        .resolve_to_key_addr(&msg.from, &curr_ts)
        .await?;

    let pending = data.mpool.pending_for(&from_a);
    let prior_messages: Vec<ChainMessage> = pending
        .map(|s| s.into_iter().map(ChainMessage::Signed).collect::<Vec<_>>())
        .unwrap_or_default();

    let ts = data.mpool.cur_tipset.lock().clone();
    // Pretend that the message is signed. This has an influence on the gas
    // cost. We obviously can't generate a valid signature. Instead, we just
    // fill the signature with zeros. The validity is not checked.
    let mut chain_msg = match from_a.protocol() {
        Protocol::Secp256k1 => ChainMessage::Signed(SignedMessage::new_unchecked(
            msg,
            Signature::new_secp256k1(vec![0; SECP_SIG_LEN]),
        )),
        Protocol::Delegated => ChainMessage::Signed(SignedMessage::new_unchecked(
            msg,
            // In Lotus, delegated signatures have the same length as SECP256k1.
            // This may or may not change in the future.
            Signature::new(SignatureType::Delegated, vec![0; SECP_SIG_LEN]),
        )),
        _ => ChainMessage::Unsigned(msg),
    };

    let res = data
        .state_manager
        .call_with_gas(&mut chain_msg, &prior_messages, Some(ts))
        .await?;
    match res.msg_rct {
        Some(rct) => {
            if rct.exit_code().value() != 0 {
                return Ok(-1);
            }
            Ok(rct.gas_used() as i64)
        }
        None => Ok(-1),
    }
}

/// Estimates the gas parameters for a given message
pub async fn gas_estimate_message_gas<DB>(
    params: Params<'_>,
    data: Ctx<DB>,
) -> Result<LotusJson<Message>, ServerError>
where
    DB: Blockstore + Send + Sync + 'static,
{
    let LotusJson((msg, spec, tsk)): LotusJson<(Message, Option<MessageSendSpec>, ApiTipsetKey)> =
        params.parse()?;

    estimate_message_gas::<DB>(&data, msg, spec, tsk)
        .await
        .map(Into::into)
}

pub async fn estimate_message_gas<DB>(
    data: &Ctx<DB>,
    msg: Message,
    _spec: Option<MessageSendSpec>,
    tsk: ApiTipsetKey,
) -> Result<Message, ServerError>
where
    DB: Blockstore + Send + Sync + 'static,
{
    let mut msg = msg;
    if msg.gas_limit == 0 {
        let gl = estimate_gas_limit::<DB>(data, msg.clone(), tsk.clone()).await?;
        msg.set_gas_limit(gl as u64);
    }
    if msg.gas_premium.is_zero() {
        let gp = estimate_gas_premium(data, 10).await?;
        msg.set_gas_premium(gp);
    }
    if msg.gas_fee_cap.is_zero() {
        let gfp = estimate_fee_cap(data, msg.clone(), 20, tsk)?;
        msg.set_gas_fee_cap(gfp);
    }
    // TODO(forest): https://github.com/ChainSafe/forest/issues/901
    //               Figure out why we always under estimate the gas
    //               calculation so we dont need to add 200000
    Ok(msg)
}