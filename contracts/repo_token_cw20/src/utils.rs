//! Scaled → underlying conversion using pool liquidity index (floor rounding).
//!
//! Rounding is **floor** so we never over-state the user's balance: the displayed/queryable
//! underlying is at most the true value; any fractional dust stays in the pool. This matches
//! pool_v2's `scaled_to_underlying_liquidity` (used for balance queries and withdraw limits).

use cosmwasm_std::{Decimal256, StdError, Uint128, Uint256};

use crate::error::ContractError;

const DECIMAL_EXP: u32 = 18;

/// Converts scaled balance to underlying (floor): underlying = scaled × liquidity_index.
///
/// Floor ensures we never show more than the user is entitled to—withdrawable/display balance
/// is slightly under the ideal amount; dust stays in the pool. Consistent with pool_v2's
/// scaled_to_underlying_liquidity (same formula and 18-decimal truncation).
pub fn scaled_to_underlying_floor(
    scaled: u128,
    liquidity_index: Decimal256,
) -> Result<u128, ContractError> {
    if liquidity_index.is_zero() {
        return Ok(0);
    }
    let scaled_d = Decimal256::from_ratio(Uint128::from(scaled), Uint128::from(1u128));
    let underlying_d = scaled_d
        .checked_mul(liquidity_index)
        .map_err(|e| ContractError::Std(StdError::generic_err(e.to_string())))?;
    let atomics = underlying_d.atomics();
    let exp = Uint256::from(10u64).pow(DECIMAL_EXP);
    let whole = atomics / exp;
    let out = whole
        .to_string()
        .parse::<u128>()
        .map_err(|_| ContractError::Overflow("underlying balance overflow".to_string()))?;
    Ok(out)
}
