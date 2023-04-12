// Copyright 2019-2023 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use forest_cli::cli::{
        send_cmd::FILAmount,
        wallet_cmd::{format_balance_string, FormattingMode},
    };
    use fvm_shared::econ::TokenAmount;
    use quickcheck_macros::quickcheck;

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
        assert!(FILAmount::from_str(amount).is_ok());
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
        assert!(FILAmount::from_str(amount).is_ok());
    }

    #[test]
    fn valid_fil_amount_with_suffix() {
        //properly parses amount with "FIL" suffix
        let amount = "1234FIL";
        assert!(FILAmount::from_str(amount).is_ok());
    }

    #[test]
    fn invalid_fil_amount() {
        //bad amount fails
        let amount = "0.0.0FIL";
        assert!(FILAmount::from_str(amount).is_err());
    }

    #[test]
    fn test_fractional_fil_amount() {
        //fil with fractional value succeeds
        let amount = "1.234FIL";
        assert!(FILAmount::from_str(amount).is_ok());
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
        let attofil_amount = TokenAmount::from_whole(1);
        assert_eq!(
            FILAmount::from_str(fil_amount).unwrap().value,
            attofil_amount
        );
    }

    #[test]
    fn invalid_fil_suffix() {
        //fails with bad suffix
        let amount = "42fiascos";
        assert!(FILAmount::from_str(amount).is_err());
    }

    #[test]
    fn malformatted_fil_suffix_test1() {
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
    fn attofil_quickcheck_test(n: u64) {
        let norm_n = n as f64 / u64::MAX as f64 * 1000.0;
        let token_amount = TokenAmount::from_atto(norm_n as u64);
        let formatted =
            format_balance_string(token_amount.clone(), FormattingMode::ExactNotFixed).unwrap();
        let parsed = FILAmount::from_str(&formatted).unwrap().value;
        assert_eq!(token_amount, parsed);
    }
}
