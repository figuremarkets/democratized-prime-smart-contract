//! Decimal helpers used by rates and other modules.

use crate::model::error::ContractError;
use cosmwasm_std::{Decimal256, Uint128};
use result_extensions::ResultExtensions;
use std::str::FromStr;

/// Converts a u128 amount to Decimal256 for ratio math (e.g. amount / index).
/// Used whenever we need to divide or multiply amounts by Decimal256 indexes/rates.
pub fn uint128_to_decimal256<T: Into<u128>>(value: T) -> Decimal256 {
    Decimal256::from_ratio(value.into(), Uint128::from(1_u64))
}

/// `ceil(numerator / denominator)` at 18 decimals; `Decimal256::checked_div` truncates.
///
/// If the truncated quotient `q` is inexact then `q × denominator < numerator` strictly, so the
/// (truncating) product check below only matches when the division was exact.
pub fn checked_div_ceil(
    numerator: Decimal256,
    denominator: Decimal256,
) -> Result<Decimal256, ContractError> {
    let quotient = numerator.checked_div(denominator)?;
    if quotient.checked_mul(denominator)? == numerator {
        return quotient.to_ok();
    }
    quotient
        .checked_add(Decimal256::raw(1))
        .map_err(ContractError::from)
}

/// Formats a decimal as a percentage string (e.g. 0.09 -> "9%") for display or error messages.
pub fn format_as_percent_string(x: Decimal256) -> Result<String, ContractError> {
    format!("{}%", x.checked_mul(Decimal256::from_str("100")?)?).to_ok()
}
