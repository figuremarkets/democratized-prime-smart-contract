//! Decimal and rounding helpers used by rates and other modules.
//!
//! Rounding choices matter: we use **ceil** when we must not under-state a requirement or
//! under-record a liability (e.g. minimum collateral, debt to record). We use **floor**/truncate
//! when converting back to integers so we never over-credit or over-charge (dust stays in the pool).

use crate::model::error::ContractError;
use cosmwasm_std::{Decimal256, Uint128, Uint256};
use result_extensions::ResultExtensions;
use std::str::FromStr;

const DECIMAL_EXP: u32 = 18;

/// Converts a u128 amount to Decimal256 for ratio math (e.g. amount / index).
/// Used whenever we need to divide or multiply amounts by Decimal256 indexes/rates.
pub fn uint128_to_decimal256<T: Into<u128>>(value: T) -> Decimal256 {
    Decimal256::from_ratio(value.into(), Uint128::from(1_u64))
}

/// Rounds up a Decimal256 to u128 (e.g. 250.3 -> 251). Returns None on overflow.
///
/// Used when the result must be at least the fractional value: e.g. minimum collateral units
/// (required_value / price_per_unit rounded up so we don't accept slightly less than required),
/// or any "minimum amount needed" derived from a decimal formula. Ceil ensures we never
/// under-require or under-record in the protocol’s favor.
pub fn decimal256_ceil_to_u128(d: Decimal256) -> Option<u128> {
    let atomics = d.atomics();
    let exp = Uint256::from(10u64).pow(DECIMAL_EXP);
    let whole = atomics / exp;
    let remainder = atomics % exp;
    let ceil = if remainder.is_zero() {
        whole
    } else {
        whole + Uint256::from(1u64)
    };
    ceil.to_string().parse::<u128>().ok()
}

/// Formats a decimal as a percentage string (e.g. 0.09 -> "9%") for display or error messages.
pub fn format_as_percent_string(x: Decimal256) -> Result<String, ContractError> {
    format!("{}%", x.checked_mul(Decimal256::from_str("100")?)?).to_ok()
}
