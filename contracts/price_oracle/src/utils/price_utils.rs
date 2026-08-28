use crate::constants::TEN;
use cosmwasm_std::Decimal256;
use democratized_prime_lib::common::ContractError;
use result_extensions::ResultExtensions;
use std::str::FromStr;

/// Scale the display USD price to per-base-unit USD: `display_price / 10^precision`.
///
/// Only used to fill deprecated [`democratized_prime_lib::price_oracle::model::AssetPriceResponseV1::price_usd`].
/// `Decimal256` floors at 1e-18, so cheap display prices at high precision can become
/// zero or a single ULP. New consumers use `display_price_usd` and `precision`.
///
/// # Example
/// display_price = 100000
/// base_precision = 3
/// display_price / 10^base_precision = 100000 / 10^3 = 100
pub fn scale_price(
    display_price: Decimal256,
    base_precision: u32,
) -> Result<Decimal256, ContractError> {
    let divisor: Decimal256 = Decimal256::from_str(TEN)?.checked_pow(base_precision)?;
    display_price.checked_div(divisor)?.to_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal256 {
        Decimal256::from_str(s).unwrap()
    }

    #[test]
    fn precision_9_btc_display_scales() {
        let scaled = scale_price(dec("100000.123"), 9).unwrap();
        assert_eq!(scaled, dec("0.000100000123"));
    }

    #[test]
    fn precision_18_integer_thousands_scales() {
        let scaled = scale_price(dec("3000"), 18).unwrap();
        assert_eq!(scaled, dec("0.000000000000003"));
    }

    #[test]
    fn precision_18_exact_one_dollar_scales() {
        let scaled = scale_price(dec("1"), 18).unwrap();
        assert_eq!(scaled, dec("0.000000000000000001"));
    }

    #[test]
    fn precision_18_half_dollar_floors_to_zero() {
        assert!(scale_price(dec("0.5"), 18).unwrap().is_zero());
    }

    #[test]
    fn precision_18_one_ninety_floors_to_one_ulp() {
        let scaled = scale_price(dec("1.9"), 18).unwrap();
        assert_eq!(scaled, dec("0.000000000000000001"));
    }

    #[test]
    fn precision_0_is_identity() {
        let price = dec("0.5");
        assert_eq!(scale_price(price, 0).unwrap(), price);
    }
}
