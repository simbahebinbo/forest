// Copyright 2019-2023 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::str::FromStr;

use forest_json::message::json::MessageJson;
use forest_rpc_client::{mpool_push_message, wallet_default_address};
use fvm_shared::{address::Address, econ::TokenAmount, message::Message, METHOD_SEND};
use num::BigInt;
use rust_decimal::prelude::*;
use rust_decimal_macros::dec;

use super::{handle_rpc_err, Config};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FILAmount {
    pub value: TokenAmount,
}

impl FromStr for FILAmount {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let suffix_idx = s.rfind(char::is_numeric);
        let (val, suffix) = match suffix_idx {
            Some(idx) => s.split_at(idx + 1),
            None => return Err(anyhow::anyhow!("failed to parse string: {}", s)),
        };
        let suffix = suffix.replace(' ', "");

        let mut multiplier = dec!(1.0);
        let prefix = if !suffix.is_empty() {
            match suffix.trim().to_lowercase().strip_suffix("fil") {
                Some("atto" | "a") => "atto",
                Some("femto") => {
                    multiplier *= dec!(1_000);
                    "femto"
                }
                Some("pico") => {
                    multiplier *= dec!(1_000_000);
                    "pico"
                }
                Some("nano") => {
                    multiplier *= dec!(1_000_000_000);
                    "nano"
                }
                Some("micro") => {
                    multiplier *= dec!(1_000_000_000_000);
                    "micro"
                }
                Some("milli") => {
                    multiplier *= dec!(1_000_000_000_000_000);
                    "milli"
                }
                Some("" | " ") => {
                    multiplier *= dec!(1_000_000_000_000_000_000);
                    ""
                }
                _ => {
                    return Err(anyhow::anyhow!("unrecognized suffix: {}", suffix));
                }
            }
        } else {
            multiplier *= dec!(1_000_000_000_000_000_000);
            ""
        };

        if val.chars().count() > 50 {
            return Err(anyhow::anyhow!(
                "string length too large: {}",
                val.chars().count()
            ));
        }

        let parsed_val = match Decimal::from_str(val) {
            Ok(value) => value,
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "failed to parse {} as a decimal number",
                    val
                ))
            }
        };

        let attofil_val = if (parsed_val * multiplier).fract() != dec!(0.0) {
            return Err(anyhow::anyhow!("invalid {}FIL value: {}", prefix, val));
        } else {
            (parsed_val * multiplier).trunc().to_u128()
        };

        let token_amount = match attofil_val {
            Some(attofil_amt) => TokenAmount::from_atto(BigInt::from(attofil_amt)),
            None => return Err(anyhow::anyhow!("invalid {}FIL value: {}", prefix, val)),
        };

        Ok(FILAmount {
            value: token_amount,
        })
    }
}

#[derive(Debug, clap::Args)]
pub struct SendCommand {
    /// optionally specify the account to send funds from (otherwise the default
    /// one will be used)
    #[arg(long)]
    from: Option<Address>,
    target_address: Address,
    /// token amount in attoFIL
    amount: FILAmount,
    /// specify gas fee cap to use in attoFIL
    #[arg(long)]
    gas_feecap: Option<BigInt>,
    /// specify gas limit in attoFIL
    #[arg(long)]
    gas_limit: Option<i64>,
    /// specify gas price to use in attoFIL
    #[arg(long)]
    gas_premium: Option<BigInt>,
}

impl SendCommand {
    pub async fn run(&self, config: Config) -> anyhow::Result<()> {
        let from: Address = if let Some(from) = self.from {
            from
        } else {
            Address::from_str(
                &wallet_default_address((), &config.client.rpc_token)
                    .await
                    .map_err(handle_rpc_err)?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No default wallet address selected. Please set a default address."
                        )
                    })?,
            )?
        };

        //TODO: update value field and update integration tests
        let message = Message {
            from,
            to: self.target_address,
            value: self.amount.value.clone(),
            method_num: METHOD_SEND,
            gas_limit: self.gas_limit.unwrap_or_default(),
            gas_fee_cap: TokenAmount::from_atto(self.gas_feecap.clone().unwrap_or_default()),
            gas_premium: TokenAmount::from_atto(self.gas_premium.clone().unwrap_or_default()),
            ..Default::default()
        };

        mpool_push_message(
            (MessageJson(message.into()), None),
            &config.client.rpc_token,
        )
        .await
        .map_err(handle_rpc_err)?;

        Ok(())
    }
}
