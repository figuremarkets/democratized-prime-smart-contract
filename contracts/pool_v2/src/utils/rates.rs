//! Kink interest rate model and index growth (scaled balances and linear accrual in time).
//!
//! Borrower rate: below kink `min + (u/kink)*(target - min)`, above kink `target + (u - kink)/(1 - kink)*(max - target)`.
//! Lender rate: `borrower_rate * utilization * (1 - reserve_factor)`.
//! Index growth: `new_index = old_index * (1 + rate * elapsed_seconds / seconds_per_year)` (linear in time).
//!
//! **Borrow vs liquidity index:** Both the lent-supply (liquidity) index and the borrow index use this same
//! linear factor. Some designs compound the borrow index as `(1+r)^t`; here both indices use `(1 + r*t)`,
//! which is consistent and keeps rounding and reserve math straightforward.
//!
//! **Rounding (scaled ↔ underlying):**
//! - **Underlying → scaled (floor)** when we *record* new lend: mint fewer repo tokens so
//!   `floor(scaled × liquidity_index) ≤ amount` and lender claims cannot exceed coins received.
//! - **Underlying → scaled (ceil)** when we *record* new borrow: scaled debt is never less than
//!   the amount lent out.
//! - **Underlying → scaled (floor)** when we *reduce* (e.g. repay amount → scaled debt to subtract):
//!   we don't remove more scaled units than the repayment entitles.
//! - **Scaled → underlying (floor/truncate)** when we read balances or debt: we never round up,
//!   so withdrawable/debt is slightly under the true value; dust stays in the pool.

use crate::model::error::{illegal_state, ContractError};
use crate::model::{RateParamsV1, ReserveStateV1};
use crate::storage::{get_reserve_state_v1, set_reserve_state_v1};
use cosmwasm_std::{ensure, Decimal256, Env, Storage, Timestamp, Uint128, Uint256};
use result_extensions::ResultExtensions;

/// Borrower APR from utilization (kink model).
/// - utilization <= kink: rate = min_rate + (utilization / kink) * (target_rate - min_rate)
/// - utilization > kink: rate = target_rate + (utilization - kink) / (1 - kink) * (max_rate - target_rate)
pub fn borrower_rate_from_utilization(
    params: &RateParamsV1,
    utilization: Decimal256,
) -> Result<Decimal256, ContractError> {
    if utilization <= params.kink_utilization {
        if params.kink_utilization.is_zero() {
            return Ok(params.min_rate);
        }
        let slope = params
            .target_rate
            .checked_sub(params.min_rate)?
            .checked_div(params.kink_utilization)?;
        let rate = params
            .min_rate
            .checked_add(slope.checked_mul(utilization)?)?;
        Ok(rate)
    } else {
        let one = Decimal256::one();
        let above_kink = one.checked_sub(params.kink_utilization)?;
        if above_kink.is_zero() {
            return Ok(params.max_rate);
        }
        let excess = utilization.checked_sub(params.kink_utilization)?;
        let slope = params
            .max_rate
            .checked_sub(params.target_rate)?
            .checked_div(above_kink)?;
        let rate = params.target_rate.checked_add(slope.checked_mul(excess)?)?;
        Ok(rate)
    }
}

/// Lender APR: borrower_rate * utilization * (1 - reserve_factor).
pub fn lender_rate_from_utilization(
    params: &RateParamsV1,
    utilization: Decimal256,
    borrower_rate: Decimal256,
) -> Result<Decimal256, ContractError> {
    let one = Decimal256::one();
    let lender_mult = utilization.checked_mul(one.checked_sub(params.reserve_factor)?)?;
    borrower_rate.checked_mul(lender_mult).map_err(Into::into)
}

/// Time elapsed in seconds (cap at 0 for past timestamps).
pub fn time_elapsed_seconds(from: Timestamp, to: Timestamp) -> u64 {
    to.seconds().saturating_sub(from.seconds())
}

/// Growth factor for index: 1 + rate * (elapsed_seconds / seconds_per_year).
pub fn index_growth_factor(
    rate: Decimal256,
    elapsed_seconds: u64,
    seconds_per_year: u64,
) -> Result<Decimal256, ContractError> {
    if elapsed_seconds == 0 {
        return Ok(Decimal256::one());
    }
    let time_fraction = Decimal256::from_ratio(
        Uint128::from(elapsed_seconds),
        Uint128::from(seconds_per_year),
    );
    let one = Decimal256::one();
    one.checked_add(rate.checked_mul(time_fraction)?)
        .map_err(Into::into)
}

/// Compute effective reserve state as of `as_of_time` (accrue interest from stored last_updated_at).
/// Read-only: does not persist. Use for queries so callers see current indexes and implied rates.
/// When accruing, adds (borrower_interest_delta - lender_interest_delta) to accrued_reserve (protocol share).
pub fn compute_effective_reserve(
    store: &dyn Storage,
    as_of_time: Timestamp,
    params: &RateParamsV1,
) -> Result<ReserveStateV1, ContractError> {
    let mut reserve = get_reserve_state_v1(store)?;
    let elapsed = time_elapsed_seconds(reserve.last_updated_at, as_of_time);
    if elapsed == 0 {
        return reserve.to_ok();
    }

    let utilization = reserve.utilization()?;
    let borrower_rate = borrower_rate_from_utilization(params, utilization)?;
    let lender_rate = lender_rate_from_utilization(params, utilization, borrower_rate)?;

    let li_factor = index_growth_factor(lender_rate, elapsed, params.seconds_per_year)?;
    let bi_factor = index_growth_factor(borrower_rate, elapsed, params.seconds_per_year)?;

    let old_li = reserve.liquidity_index;
    let old_bi = reserve.borrow_index;
    let new_li = reserve.liquidity_index.checked_mul(li_factor)?;
    let new_bi = reserve.borrow_index.checked_mul(bi_factor)?;

    reserve.liquidity_index = new_li;
    reserve.borrow_index = new_bi;
    reserve.last_updated_at = as_of_time;

    // Protocol reserve accrual: (borrower interest - lender interest) this period, in underlying units.
    // Indexes only grow, so new >= old; underflow would indicate a bug.
    let old_borrow = scaled_to_underlying_borrow(reserve.total_scaled_borrow, old_bi)?;
    let new_borrow =
        scaled_to_underlying_borrow(reserve.total_scaled_borrow, reserve.borrow_index)?;
    let old_liq = scaled_to_underlying_liquidity(reserve.total_scaled_liquidity, old_li)?;
    let new_liq =
        scaled_to_underlying_liquidity(reserve.total_scaled_liquidity, reserve.liquidity_index)?;
    let borrower_delta = new_borrow.checked_sub(old_borrow).ok_or_else(|| {
        illegal_state("Reserve accrual: borrow underlying decreased (index invariant)")
    })?;
    let lender_delta = new_liq.checked_sub(old_liq).ok_or_else(|| {
        illegal_state("Reserve accrual: liquidity underlying decreased (index invariant)")
    })?;
    // reserve_delta can be 0 when lender_delta >= borrower_delta (rounding with tiny borrow / large liquidity).
    let reserve_delta = borrower_delta.saturating_sub(lender_delta);
    reserve.accrued_reserve = reserve.accrued_reserve.saturating_add(reserve_delta);

    Ok(reserve)
}

/// Update reserve indexes to current block time (accrue interest), then save and return new reserve.
pub fn update_reserve_indexes(
    store: &mut dyn Storage,
    env: &Env,
    params: &RateParamsV1,
) -> Result<ReserveStateV1, ContractError> {
    let reserve = compute_effective_reserve(store, env.block.time, params)?;
    set_reserve_state_v1(store, &reserve)?;
    Ok(reserve)
}

/// Convert underlying amount to scaled liquidity (floor): scaled = underlying / liquidity_index.
///
/// Use when *reducing* liquidity (e.g. withdraw, transfer): convert requested underlying to scaled
/// units to deduct. Floor ensures we never deduct more scaled than the request entitles. For
/// Withdraw, the pool must send scaled_to_underlying(scaled)—not the requested amount—so coins
/// sent match liquidity deducted; otherwise floor(amount/index)×index < amount would over-credit
/// the user and leak from the pool.
pub fn underlying_to_scaled_liquidity(
    underlying: u128,
    liquidity_index: Decimal256,
) -> Result<u128, ContractError> {
    ensure!(
        !liquidity_index.is_zero(),
        illegal_state("liquidity_index is zero")
    );

    let underlying_d = Decimal256::from_ratio(Uint128::from(underlying), Uint128::from(1u128));
    let scaled_d = underlying_d.checked_div(liquidity_index)?;
    // Truncate to u128: atomics are in 10^18, so integer part = atomics / 10^18
    let atomics = scaled_d.atomics();
    let exp = Uint256::from(10u64).pow(18u32);
    let whole = atomics.checked_div(exp)?;
    let out = uint256_to_u128(whole).map_err(|_| ContractError::IllegalStateError {
        message: "scaled liquidity overflow".to_string(),
    })?;
    Ok(out)
}

/// Convert underlying amount to scaled borrow (floor): scaled = underlying / borrow_index.
///
/// Use when *reducing* debt: e.g. Repay execute converts the repay amount (underlying) to scaled
/// debt to subtract. Floor ensures we subtract at most the entitled amount—we never remove more
/// scaled debt than the repayment covers (no over-reduction of debt).
pub fn underlying_to_scaled_borrow(
    underlying: u128,
    borrow_index: Decimal256,
) -> Result<u128, ContractError> {
    ensure!(
        !borrow_index.is_zero(),
        illegal_state("borrow_index is zero")
    );

    let underlying_d = Decimal256::from_ratio(Uint128::from(underlying), Uint128::from(1u128));
    let scaled_d = underlying_d.checked_div(borrow_index)?;
    let atomics = scaled_d.atomics();
    let exp = Uint256::from(10u64).pow(18u32);
    let whole = atomics.checked_div(exp)?;
    let out = uint256_to_u128(whole).map_err(|_| ContractError::IllegalStateError {
        message: "scaled borrow overflow".to_string(),
    })?;
    Ok(out)
}

/// Convert underlying amount to scaled borrow (ceil). Use when *adding* new borrows (Borrow execute).
///
/// We send `underlying` to the user and record scaled = ceil(underlying / borrow_index). Then
/// (scaled × borrow_index) ≥ underlying, so our recorded debt is never less than what we lent.
/// Ceil avoids under-recording when borrow_index has accrued above 1.
pub fn underlying_to_scaled_borrow_ceil(
    underlying: u128,
    borrow_index: Decimal256,
) -> Result<u128, ContractError> {
    ensure!(
        !borrow_index.is_zero(),
        illegal_state("borrow_index is zero")
    );

    let underlying_d = Decimal256::from_ratio(Uint128::from(underlying), Uint128::from(1u128));
    let scaled_d = underlying_d.checked_div(borrow_index)?;
    let atomics = scaled_d.atomics();
    let exp = Uint256::from(10u64).pow(18u32);
    let whole = atomics.checked_div(exp)?;
    let remainder = atomics.checked_rem(exp)?; // same as `atomics % exp`
    let whole_ceil = if remainder.is_zero() {
        whole
    } else {
        whole.checked_add(Uint256::from(1u64))?
    };
    let out = uint256_to_u128(whole_ceil).map_err(|_| illegal_state("scaled borrow overflow"))?;
    Ok(out)
}

/// Convert scaled liquidity to underlying (floor/truncate): underlying = scaled × liquidity_index.
///
/// Used for balance queries and withdraw limits: "how much can the user withdraw?" We truncate
/// (drop the fractional part), so the result is never more than the true value—withdrawable is
/// slightly under the ideal amount; dust stays in the pool. Prevents over-withdrawal.
pub fn scaled_to_underlying_liquidity(
    scaled: u128,
    liquidity_index: Decimal256,
) -> Result<u128, ContractError> {
    let scaled_d = Decimal256::from_ratio(Uint128::from(scaled), Uint128::from(1u128));
    let underlying_d = scaled_d.checked_mul(liquidity_index)?;
    let atomics = underlying_d.atomics();
    let exp = Uint256::from(10u64).pow(18u32);
    let whole = atomics.checked_div(exp)?;
    let out = uint256_to_u128(whole).map_err(|_| illegal_state("underlying liquidity overflow"))?;
    Ok(out)
}

/// Convert scaled borrow to underlying (floor/truncate): underlying = scaled × borrow_index.
///
/// Used for debt queries, repay checks, and liquidation: "how much does the user owe?" We truncate
/// so reported debt is never more than the true value—we never over-state what's owed. Repay and
/// liquidation use this value, so rounding is in the protocol’s favor (slightly under true debt).
pub fn scaled_to_underlying_borrow(
    scaled: u128,
    borrow_index: Decimal256,
) -> Result<u128, ContractError> {
    let scaled_d = Decimal256::from_ratio(Uint128::from(scaled), Uint128::from(1u128));
    let underlying_d = scaled_d.checked_mul(borrow_index)?;
    let atomics = underlying_d.atomics();
    let exp = Uint256::from(10u64).pow(18u32);
    let whole = atomics.checked_div(exp)?;
    let out = uint256_to_u128(whole).map_err(|_| illegal_state("underlying borrow overflow"))?;
    Ok(out)
}

/// Total liquidity, total borrow, and cash in u128 (floor), for execute limits.
/// Cash = **`saturating_sub`**(`total_liquidity`, `total_borrow`) **− `deficit_underlying`** (each step
/// saturates at zero), matching the spirit of [`ReserveStateV1::cash`] (saturating on `Decimal256`).
/// Callers cap borrows/withdraws with **`amount <= cash`** and reject zero amounts where required.
/// Totals still use the same `scaled_to_underlying_*` floor conversions as
/// [`ReserveStateV1::total_liquidity`] / [`ReserveStateV1::total_borrow`] in u128 form.
pub fn reserve_totals_and_cash_u128(
    reserve: &ReserveStateV1,
) -> Result<(u128, u128, u128), ContractError> {
    let total_liquidity_u128 =
        scaled_to_underlying_liquidity(reserve.total_scaled_liquidity, reserve.liquidity_index)?;
    let total_borrow_u128 =
        scaled_to_underlying_borrow(reserve.total_scaled_borrow, reserve.borrow_index)?;
    let cash = total_liquidity_u128
        .saturating_sub(total_borrow_u128)
        .saturating_sub(reserve.deficit_underlying);
    Ok((total_liquidity_u128, total_borrow_u128, cash))
}

/// Pro-rata supplier loss: multiply `liquidity_index` by **`(L − loss) / L`** where **`L`** is
/// [`ReserveStateV1::total_liquidity`] (**exact** `total_scaled_liquidity × liquidity_index` in
/// [`Decimal256`]) and **`loss`** is the integer underlying amount in the same units (as
/// [`Decimal256::from_ratio`]). **`total_scaled_liquidity` unchanged.**
///
/// **Rounding:** [`Decimal256::checked_div`] truncates the factor toward zero, so the updated
/// index is never rounded **up** versus the exact \((L-\mathrm{loss})/L\); suppliers’ aggregate
/// underlying claim ends up micro-conservative (slightly under the exact split), which avoids
/// over-promising liquidity after the loss.
///
/// When folding booked bad debt, caller must also subtract the same **`loss`** from
/// **`deficit_underlying`** in the same transaction (see **`SocializeDeficit`** execute).
pub fn apply_pro_rata_liquidity_index_haircut(
    reserve: &mut ReserveStateV1,
    underlying_loss: u128,
) -> Result<(), ContractError> {
    if underlying_loss == 0 {
        return Ok(());
    }
    let l = reserve.total_liquidity()?;
    ensure!(
        !l.is_zero(),
        illegal_state("cannot apply liquidity index haircut: total_liquidity is zero")
    );
    let d = Decimal256::from_ratio(Uint128::from(underlying_loss), Uint128::one());
    ensure!(
        d < l,
        illegal_state(
            "bad debt loss meets or exceeds total_liquidity; cannot apply index-only haircut"
        )
    );
    let factor = l.checked_sub(d)?.checked_div(l)?;
    reserve.liquidity_index = reserve.liquidity_index.checked_mul(factor)?;
    Ok(())
}

/// Convert Uint256 to u128; errors if value exceeds u128::MAX.
/// Used after Decimal256 atomics / 10^18 so we can safely store or return u128 amounts.
fn uint256_to_u128(v: Uint256) -> Result<u128, ()> {
    v.to_string().parse().map_err(|_| ())
}

/// Lender/borrower APR strings, indexes, and utilization as emitted on the response by
/// [`crate::utils::WithRates::attach_rates`]
/// (`lend_rate`, `borrow_rate`, `liquidity_index`, `borrow_index`, `utilization`).
pub fn lend_borrow_rate_attribute_values(
    reserve: &ReserveStateV1,
    rate_params: &RateParamsV1,
) -> Result<(String, String, String, String, String), ContractError> {
    let utilization = reserve.utilization()?;
    let new_borrower_rate = borrower_rate_from_utilization(rate_params, utilization)?;
    let new_lender_rate =
        lender_rate_from_utilization(rate_params, utilization, new_borrower_rate)?;
    Ok((
        new_lender_rate.to_string(),
        new_borrower_rate.to_string(),
        reserve.liquidity_index.to_string(),
        reserve.borrow_index.to_string(),
        utilization.to_string(),
    ))
}
