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
            "femto" => dec!(1e3),
            "pico" => dec!(1e6),
            "nano" => dec!(1e9),
            "micro" => dec!(1e12),
            "milli" => dec!(1e15),
            "" => dec!(1e18),
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
            "Could not covert amount input to send into an attoFIL value (note that negative FIL amounts are not valid).",
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use fvm_shared::econ::TokenAmount;
    use quickcheck_macros::quickcheck;

    use super::*;
    use crate::cli::wallet_cmd::{format_balance_string, FormattingMode};

    #[test]
    fn invalid_attofil_amount() {
        //attoFIL with fractional value fails (fractional FIL values allowed)
        let amount = "1.234attofil";
        assert!(FILAmount::from_str(amount).is_err());
    }

    #[test]
    fn valid_attofil_amount_test1() {
        //valid attofil amount passes
        let amount = "1234 attofil";
        assert_eq!(
            FILAmount::from_str(amount).unwrap().value,
            TokenAmount::from_atto(1234)
        );
    }

    #[test]
    fn valid_attofil_amount_test2() {
        //valid attofil amount passes
        let amount = "1234 afil";
        assert_eq!(
            FILAmount::from_str(amount).unwrap().value,
            TokenAmount::from_atto(1234)
        );
    }

    #[test]
    fn valid_attofil_amount_test3() {
        //valid attofil amount passes
        let amount = "1234 a fil";
        assert_eq!(
            FILAmount::from_str(amount).unwrap().value,
            TokenAmount::from_atto(1234)
        );
    }

    #[test]
    fn valid_attofil_amount_test4() {
        //valid attofil amount passes
        let amount = "1234 a";
        assert_eq!(
            FILAmount::from_str(amount).unwrap().value,
            TokenAmount::from_atto(1234)
        );
    }

    #[test]
    fn suffix_with_no_amount() {
        //fails if no amount specified
        let amount = "fil";
        assert!(FILAmount::from_str(amount).is_err());
    }
    #[test]
    fn valid_fil_amount_without_suffix() {
        //defaults to FIL if no suffix is provided
        let amount = "1234";
        assert_eq!(
            FILAmount::from_str(amount).unwrap().value,
            TokenAmount::from_whole(1234)
        );
    }

    #[test]
    fn valid_fil_amount_with_suffix() {
        //properly parses amount with "FIL" suffix
        let amount = "1234FIL";
        assert_eq!(
            FILAmount::from_str(amount).unwrap().value,
            TokenAmount::from_whole(1234)
        );
    }

    #[test]
    fn invalid_fil_amount() {
        //bad amount fails
        let amount = "0.0.0FIL";
        assert!(FILAmount::from_str(amount).is_err());
    }

    #[test]
    fn test_fractional_fil_amount() {
        //properly parses fil with fractional value
        let amount = "1.234FIL";
        assert_eq!(
            FILAmount::from_str(amount).unwrap().value,
            TokenAmount::from_atto(1_234_000_000_000_000_000i64)
        );
    }

    #[test]
    fn fil_amount_too_long() {
        //fil amount with length>50 fails
        let amount = "100000000000000000000000000000000000000000000000000FIL";
        assert!(FILAmount::from_str(amount).is_err());
    }

    #[test]
    fn convert_fil_to_attofil() {
        //expected attofil amount matches actual amount after conversion from FIL
        let fil_amount = "1FIL";
        assert_eq!(
            FILAmount::from_str(fil_amount).unwrap().value,
            TokenAmount::from_whole(1)
        );
    }

    #[test]
    fn invalid_fil_suffix() {
        //fails with bad suffix
        let amount = "42fiascos";
        assert!(FILAmount::from_str(amount).is_err());
    }

    #[test]
    fn malformatted_fil_suffix_test() {
        //fails with bad suffix
        let amount = "42 fem to fil";
        assert!(FILAmount::from_str(amount).is_err());
    }

    #[test]
    fn negative_fil_value() {
        //fails with negative value
        let amount = "-1FIL";
        assert!(FILAmount::from_str(amount).is_err());
    }

    #[quickcheck]
    fn fil_quickcheck_test(n: u64) {
        let token_amount = TokenAmount::from_atto(n);
        let formatted =
            format_balance_string(token_amount.clone(), FormattingMode::ExactNotFixed).unwrap();
        let parsed = FILAmount::from_str(&formatted).unwrap().value;
        assert_eq!(token_amount, parsed);
    }

    #[quickcheck]
    fn scaled_fil_quickcheck_test(n: u64, rand_num: u32) {
        let scaled_n = n % u64::pow(10, rand_num % 20);
        let token_amount = TokenAmount::from_atto(scaled_n);
        let formatted =
            format_balance_string(token_amount.clone(), FormattingMode::ExactNotFixed).unwrap();
        let parsed = FILAmount::from_str(&formatted).unwrap().value;
        assert_eq!(token_amount, parsed);
    }
}
