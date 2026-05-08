//! Shared helper: assert assets − liabilities tie out after any execute that changes reserve state.
//!
//! Identity: Assets = cash + total_borrow, Liabilities = total_liquidity + accrued_reserve.
//! So assets − liabilities = 0 ⟺ cash = total_liquidity + accrued_reserve − total_borrow − **deficit_underlying**
//! (`implied_cash`). Bad-debt liquidation books `deficit_underlying` so free cash matches bank reality.
//! We always assert implied_cash ≥ 0 (solvency). When expected_implied_cash is provided, we also
//! assert |implied_cash − expected_implied_cash| ≤ tolerance to catch drift from rounding or bugs.
//!
//! **When is tolerance needed vs not?**
//! * **Tolerance = 0**: Use when you only need solvency (implied_cash ≥ 0) or when state is
//!   synthetic and there is no scaled↔underlying conversion (e.g. hand-built reserve in tests).
//! * **Tolerance > 0**: Use whenever expected_implied_cash is derived from flows (e.g. cash_before −
//!   amount_sent) and the contract state uses scaled amounts + indexes. Drift is then expected and
//!   acceptable for the following reasons:
//!   - **Scaled ↔ underlying**: The contract uses floor when reducing (withdraw, repay) and when
//!     recording new lend supply; borrow still uses ceil when adding debt. Aggregate totals can
//!     still differ from a naive sum of flows by a few base units from borrow-side rounding and
//!     index truncation.
//!   - **Index rounding**: total_liquidity and total_borrow are computed as scaled × index with
//!     truncation; small rounding can accumulate.
//!     A tolerance of a few base units (e.g. 10) is enough to allow this acceptable drift while still
//!     failing on real accounting bugs.

use crate::model::error::ContractError;
use crate::model::ReserveStateV1;
use crate::storage::get_reserve_state_v1;
use crate::utils::{scaled_to_underlying_borrow, scaled_to_underlying_liquidity};
use cosmwasm_std::Storage;

/// Assert assets − liabilities tie out: implied_cash ≥ 0 and optionally within tolerance of expected.
///
/// * implied_cash = total_liquidity + accrued_reserve − total_borrow − deficit_underlying (must be ≥ 0).
/// * If `expected_implied_cash` is `Some(expected)`, asserts `|implied_cash − expected| ≤ tolerance`
///   so drift from rounding or accounting bugs is bounded.
///
/// Returns (total_liquidity, total_borrow, accrued_reserve) for use in conservation checks.
pub fn assert_assets_liabilities_tie_out(
    reserve: &ReserveStateV1,
    step_name: &str,
) -> Result<(u128, u128, u128), ContractError> {
    assert_assets_liabilities_tie_out_with_tolerance(reserve, step_name, None, 0)
}

/// Same as `assert_assets_liabilities_tie_out` but optionally assert implied_cash is within
/// `tolerance` of `expected_implied_cash`. Use tolerance > 0 when expected is flow-derived and
/// state uses scaled amounts (see module doc for why drift is acceptable).
pub fn assert_assets_liabilities_tie_out_with_tolerance(
    reserve: &ReserveStateV1,
    step_name: &str,
    expected_implied_cash: Option<u128>,
    tolerance: u128,
) -> Result<(u128, u128, u128), ContractError> {
    let total_liquidity =
        scaled_to_underlying_liquidity(reserve.total_scaled_liquidity, reserve.liquidity_index)?;
    let total_borrow =
        scaled_to_underlying_borrow(reserve.total_scaled_borrow, reserve.borrow_index)?;
    let implied_cash = total_liquidity
        .saturating_add(reserve.accrued_reserve)
        .saturating_sub(total_borrow)
        .saturating_sub(reserve.deficit_underlying);

    assert!(
        total_liquidity.saturating_add(reserve.accrued_reserve)
            >= total_borrow.saturating_add(reserve.deficit_underlying),
        "{}: assets - liabilities must balance (implied_cash >= 0); liq={} bor={} accrued_reserve={} deficit_underlying={}",
        step_name,
        total_liquidity,
        total_borrow,
        reserve.accrued_reserve,
        reserve.deficit_underlying
    );

    if let Some(expected) = expected_implied_cash {
        let drift = implied_cash.abs_diff(expected);
        assert!(
            drift <= tolerance,
            "{}: implied_cash drift too large; implied_cash={} expected={} drift={} tolerance={}",
            step_name,
            implied_cash,
            expected,
            drift,
            tolerance
        );
    }

    Ok((total_liquidity, total_borrow, reserve.accrued_reserve))
}

/// Load reserve from storage and assert assets-liabilities tie out. Use at end of tests that mutate reserve.
pub fn assert_reserve_assets_liabilities_tie_out(
    storage: &dyn Storage,
    step_name: &str,
) -> Result<(), ContractError> {
    let reserve = get_reserve_state_v1(storage)?;
    assert_assets_liabilities_tie_out(&reserve, step_name).map(|_| ())
}

/// Load reserve from storage and assert tie out with optional expected implied_cash and tolerance.
pub fn assert_reserve_assets_liabilities_tie_out_with_tolerance(
    storage: &dyn Storage,
    step_name: &str,
    expected_implied_cash: Option<u128>,
    tolerance: u128,
) -> Result<(), ContractError> {
    let reserve = get_reserve_state_v1(storage)?;
    assert_assets_liabilities_tie_out_with_tolerance(
        &reserve,
        step_name,
        expected_implied_cash,
        tolerance,
    )
    .map(|_| ())
}
