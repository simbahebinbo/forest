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
            None => anyhow::bail!("failed to parse string: {}", s),
        };

        let suffix = suffix.trim().to_lowercase();
        let multiplier = match suffix.strip_suffix("fil").map(str::trim).unwrap_or(&suffix) {
            "atto" | "a" => dec!(1e0),
            "femto" => {
                dec!(1e3)
            }
            "pico" => {
                dec!(1e6)
            }
            "nano" => {
                dec!(1e9)
            }
            "micro" => {
                dec!(1e12)
            }
            "milli" => {
                dec!(1e15)
            }
            "" => {
                dec!(1e18)
            }
            _ => {
                anyhow::bail!("unrecognized suffix: {}", suffix);
            }
        };

        // string length check to match Lotus logic
        if val.chars().count() > 50 {
            anyhow::bail!("string length too large: {}", val.chars().count());
        }

        let parsed_val = Decimal::from_str(val)
            .map_err(|_| anyhow::anyhow!("failed to parse {} as a decimal number", val))?;

        let val = parsed_val * multiplier;
        if val.normalize().scale() > 0 {
            anyhow::bail!("{} must convert to a whole attoFIL value", &s);
        }
        let attofil_val = val.trunc().to_u128().ok_or(anyhow::Error::msg(
            "Could not covert amount input to send into an attoFIL value.",
        ))?;

        let token_amount = TokenAmount::from_atto(BigInt::from(attofil_val));

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
