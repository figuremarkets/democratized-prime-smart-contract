//! Tests for kink rate model, lender rate, index growth, time elapsed, `reserve_totals_and_cash_u128`,
//! and `apply_pro_rata_liquidity_index_haircut` (bad-debt supplier loss).

use crate::model::error::ContractError;
use crate::model::{FeeModelV1, RateParamsV1, ReserveStateV1};
use crate::utils::rates::{
    apply_pro_rata_liquidity_index_haircut, borrower_rate_from_utilization, index_growth_factor,
    lender_rate_from_utilization, reserve_totals_and_cash_u128, time_elapsed_seconds,
};
use cosmwasm_std::{Decimal256, Timestamp};
use std::str::FromStr;

/// Spreadsheet params: target 9%, min 3.25%, max 20%, kink 90%, reserve 0.5%, 31_536_000 s/year.
fn spreadsheet_rate_params() -> RateParamsV1 {
    RateParamsV1 {
        target_rate: Decimal256::from_str("0.09").unwrap(),
        min_rate: Decimal256::from_str("0.0325").unwrap(),
        max_rate: Decimal256::from_str("0.20").unwrap(),
        kink_utilization: Decimal256::from_str("0.90").unwrap(),
        reserve_factor: Decimal256::from_str("0.005").unwrap(),
        fee_model: Default::default(),
        flat_fee_apr: Decimal256::zero(),
        seconds_per_year: 31_536_000,
    }
}

/// Assert two Decimal256 are within 1e-10 (for spreadsheet rounding tolerance).
fn assert_near(a: Decimal256, b: Decimal256, msg: &str) {
    let diff = if a > b {
        a.checked_sub(b).unwrap()
    } else {
        b.checked_sub(a).unwrap()
    };
    let eps = Decimal256::from_str("0.0000000001").unwrap();
    assert!(diff < eps, "{}: {} vs {}", msg, a, b);
}

// --- Borrower rate (kink model) ---

#[test]
fn borrower_rate_at_zero_utilization_equals_min_rate() {
    let params = spreadsheet_rate_params();
    let u = Decimal256::zero();
    let rate = borrower_rate_from_utilization(&params, u).unwrap();
    assert_near(
        rate,
        params.min_rate,
        "at 0% utilization rate should be min_rate (3.25%)",
    );
}

/// Borrow index accrues at min_rate even with zero borrows (utilization 0). A spreadsheet that
/// assumes "borrow index = 1 until first borrow" will show different index/scaled values.
#[test]
fn index_growth_at_min_rate_with_elapsed_time_is_above_one() {
    let params = spreadsheet_rate_params();
    let one = Decimal256::one();
    let factor = index_growth_factor(params.min_rate, 86_400, params.seconds_per_year).unwrap(); // 1 day
    assert!(
        factor > one,
        "borrow index growth factor at min_rate over 1 day should be > 1 (actual {})",
        factor
    );
}

#[test]
fn borrower_rate_at_kink_equals_target_rate() {
    let params = spreadsheet_rate_params();
    let u = params.kink_utilization; // 90%
    let rate = borrower_rate_from_utilization(&params, u).unwrap();
    assert_near(
        rate,
        params.target_rate,
        "at kink (90%) rate should be target (9%)",
    );
}

#[test]
fn borrower_rate_at_full_utilization_equals_max_rate() {
    let params = spreadsheet_rate_params();
    let u = Decimal256::one();
    let rate = borrower_rate_from_utilization(&params, u).unwrap();
    assert_near(
        rate,
        params.max_rate,
        "at 100% utilization rate should be max (20%)",
    );
}

#[test]
fn borrower_rate_at_80_percent_near_spreadsheet() {
    let params = spreadsheet_rate_params();
    let u = Decimal256::from_str("0.80").unwrap();
    let rate = borrower_rate_from_utilization(&params, u).unwrap();
    // Formula: min + (u/kink)*(target - min) = 0.0325 + (8/9)*0.0575 ≈ 0.083611...
    let expected = Decimal256::from_str("0.08361111111111111").unwrap();
    assert_near(rate, expected, "borrower rate at 80% utilization");
}

#[test]
fn borrower_rate_below_kink_increases_linearly() {
    let params = spreadsheet_rate_params();
    let u50 = Decimal256::from_str("0.50").unwrap();
    let u90 = params.kink_utilization;
    let r50 = borrower_rate_from_utilization(&params, u50).unwrap();
    let r90 = borrower_rate_from_utilization(&params, u90).unwrap();
    assert!(r50 > params.min_rate && r50 < params.target_rate);
    assert_near(r90, params.target_rate, "at kink rate should be target");
    // Rate = min + (u/kink)*(target - min). At 50% util with 90% kink: (0.5/0.9) * delta ≈ 0.06444
    let expected_r50 = Decimal256::from_str("0.064444444444444444").unwrap();
    assert_near(r50, expected_r50, "50% util ~ linear formula");
}

#[test]
fn borrower_rate_above_kink_increases_steeply() {
    let params = spreadsheet_rate_params();
    let u95 = Decimal256::from_str("0.95").unwrap();
    let r95 = borrower_rate_from_utilization(&params, u95).unwrap();
    assert!(r95 > params.target_rate && r95 < params.max_rate);
}

// --- Lender rate ---

#[test]
fn lender_rate_at_80_percent_utilization_near_spreadsheet() {
    let params = spreadsheet_rate_params();
    let u = Decimal256::from_str("0.80").unwrap();
    let borrower_rate = borrower_rate_from_utilization(&params, u).unwrap();
    let lender_rate = lender_rate_from_utilization(&params, u, borrower_rate).unwrap();
    // lender = borrower_rate * u * (1 - reserve_factor); at 80% ≈ 0.06655444...
    let expected = Decimal256::from_str("0.066554444444444443").unwrap();
    assert_near(lender_rate, expected, "lender rate at 80% utilization");
}

#[test]
fn lender_rate_at_zero_utilization_is_zero() {
    let params = spreadsheet_rate_params();
    let u = Decimal256::zero();
    let borrower_rate = borrower_rate_from_utilization(&params, u).unwrap();
    let lender_rate = lender_rate_from_utilization(&params, u, borrower_rate).unwrap();
    assert!(lender_rate.is_zero(), "lender rate at 0% util should be 0");
}

#[test]
fn lender_rate_flat_spread_mode_matches_sheet_identity() {
    let mut params = spreadsheet_rate_params();
    params.fee_model = FeeModelV1::FlatBorrowSpread;
    params.flat_fee_apr = Decimal256::from_str("0.005").unwrap();
    let u = Decimal256::from_str("0.95").unwrap();
    let borrower_rate = borrower_rate_from_utilization(&params, u).unwrap();
    let lender_rate = lender_rate_from_utilization(&params, u, borrower_rate).unwrap();
    let protocol_rate = params.flat_fee_apr.checked_mul(u).unwrap();
    let borrower_flow_rate = borrower_rate.checked_mul(u).unwrap();
    // borrower_rate * U == lender_rate + protocol_rate
    assert_near(
        borrower_flow_rate,
        lender_rate.checked_add(protocol_rate).unwrap(),
        "flat spread split must tie out",
    );
}

#[test]
fn rate_params_flat_spread_rejects_fee_above_min_rate() {
    let mut params = spreadsheet_rate_params();
    params.fee_model = FeeModelV1::FlatBorrowSpread;
    params.flat_fee_apr = Decimal256::from_str("0.04").unwrap();
    let err = params.validate().unwrap_err();
    match err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("flat_fee_apr"));
            assert!(message.contains("min_rate"));
        }
        _ => panic!("expected IllegalArgumentError"),
    }
}

#[test]
fn rate_params_reserve_factor_rejects_non_zero_flat_fee() {
    let mut params = spreadsheet_rate_params();
    params.fee_model = FeeModelV1::ReserveFactor;
    params.flat_fee_apr = Decimal256::from_str("0.001").unwrap();
    let err = params.validate().unwrap_err();
    match err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("flat_fee_apr"));
            assert!(message.contains("reserve_factor"));
        }
        _ => panic!("expected IllegalArgumentError"),
    }
}

// --- Index growth ---

#[test]
fn index_growth_factor_zero_elapsed_is_one() {
    let params = spreadsheet_rate_params();
    let rate = Decimal256::from_str("0.09").unwrap();
    let factor = index_growth_factor(rate, 0, params.seconds_per_year).unwrap();
    assert_eq!(factor, Decimal256::one());
}

#[test]
fn index_growth_factor_one_year_at_9_percent() {
    let rate = Decimal256::from_str("0.09").unwrap();
    let factor = index_growth_factor(rate, 31_536_000, 31_536_000).unwrap();
    let expected = Decimal256::from_str("1.09").unwrap();
    assert_near(factor, expected, "1 + 9% over 1 year");
}

#[test]
fn index_growth_factor_half_year_at_6_percent() {
    let rate = Decimal256::from_str("0.06").unwrap();
    let half_year = 31_536_000 / 2;
    let factor = index_growth_factor(rate, half_year, 31_536_000).unwrap();
    let expected = Decimal256::from_str("1.03").unwrap(); // 1 + 3%
    assert_near(factor, expected, "1 + 6% * 0.5 year");
}

#[test]
fn time_elapsed_seconds_forward_and_backward() {
    let from = Timestamp::from_seconds(1000);
    let to = Timestamp::from_seconds(1500);
    assert_eq!(time_elapsed_seconds(from, to), 500);
    assert_eq!(time_elapsed_seconds(to, from), 0);
}

// --- reserve_totals_and_cash_u128 ---

fn reserve(
    liq_index: Decimal256,
    borr_index: Decimal256,
    scaled_liq: u128,
    scaled_borr: u128,
) -> ReserveStateV1 {
    ReserveStateV1 {
        liquidity_index: liq_index,
        borrow_index: borr_index,
        last_updated_at: Timestamp::from_seconds(0),
        total_scaled_liquidity: scaled_liq,
        total_scaled_borrow: scaled_borr,
        accrued_reserve: 0,
        deficit_underlying: 0,
    }
}

#[test]
fn reserve_totals_and_cash_u128_zero_liquidity_zero_borrow() {
    let r = reserve(Decimal256::one(), Decimal256::one(), 0, 0);
    let (liq, bor, cash) = reserve_totals_and_cash_u128(&r).unwrap();
    assert_eq!(liq, 0);
    assert_eq!(bor, 0);
    assert_eq!(cash, 0);
}

#[test]
fn reserve_totals_and_cash_u128_some_liquidity_zero_borrow() {
    let r = reserve(Decimal256::one(), Decimal256::one(), 1_000_000, 0);
    let (liq, bor, cash) = reserve_totals_and_cash_u128(&r).unwrap();
    assert_eq!(liq, 1_000_000);
    assert_eq!(bor, 0);
    assert_eq!(cash, 1_000_000);
}

#[test]
fn reserve_totals_and_cash_u128_liquidity_and_borrow_cash_is_difference() {
    let r = reserve(Decimal256::one(), Decimal256::one(), 1_000_000, 300_000);
    let (liq, bor, cash) = reserve_totals_and_cash_u128(&r).unwrap();
    assert_eq!(liq, 1_000_000);
    assert_eq!(bor, 300_000);
    assert_eq!(cash, 700_000);
}

#[test]
fn reserve_totals_and_cash_u128_with_index_above_one_floors_to_u128() {
    // Indexes > 1: scaled * index has fractional part; u128 result is floor (truncate).
    // 1000 * 1.5 = 1500 exactly; 1000 * 1.7 = 1700 exactly.
    let liq_index = Decimal256::from_str("1.5").unwrap();
    let borr_index = Decimal256::from_str("1.7").unwrap();
    let r = reserve(liq_index, borr_index, 1_000, 100);
    let (liq_u128, bor_u128, cash_u128) = reserve_totals_and_cash_u128(&r).unwrap();
    assert_eq!(liq_u128, 1_500); // 1000 * 1.5
    assert_eq!(bor_u128, 170); // 100 * 1.7
    assert_eq!(cash_u128, 1_330); // 1500 - 170
}

#[test]
fn reserve_totals_and_cash_u128_deficit_reduces_cash() {
    let mut r = reserve(Decimal256::one(), Decimal256::one(), 1_000_000, 300_000);
    r.deficit_underlying = 50_000;
    let (liq, bor, cash) = reserve_totals_and_cash_u128(&r).unwrap();
    assert_eq!(liq, 1_000_000);
    assert_eq!(bor, 300_000);
    assert_eq!(cash, 650_000);
}

#[test]
fn reserve_totals_and_cash_u128_borrow_exceeds_liquidity_yields_zero_cash() {
    // Floored totals: borrow > liquidity → cash saturates to 0; callers reject amount > cash.
    let r = reserve(Decimal256::one(), Decimal256::one(), 100, 101);
    let (liq, bor, cash) = reserve_totals_and_cash_u128(&r).unwrap();
    assert_eq!(liq, 100);
    assert_eq!(bor, 101);
    assert_eq!(cash, 0);
}

#[test]
fn reserve_totals_and_cash_u128_deficit_exceeds_free_yields_zero_cash() {
    let mut r = reserve(Decimal256::one(), Decimal256::one(), 1_000_000, 300_000);
    r.deficit_underlying = 800_000;
    let (liq, bor, cash) = reserve_totals_and_cash_u128(&r).unwrap();
    assert_eq!(liq, 1_000_000);
    assert_eq!(bor, 300_000);
    assert_eq!(cash, 0);
}

// --- apply_pro_rata_liquidity_index_haircut (bad-debt supplier loss) ---

#[test]
fn apply_pro_rata_liquidity_index_haircut_scales_index() {
    let mut r = reserve(Decimal256::one(), Decimal256::one(), 1_000_000, 0);
    apply_pro_rata_liquidity_index_haircut(&mut r, 50_000).unwrap();
    // I' = 1 * (1_000_000 - 50_000) / 1_000_000 = 0.95
    let exp = Decimal256::from_ratio(95u128, 100u128);
    assert_eq!(r.liquidity_index, exp);
}

/// Loss must be **strictly** less than total liquidity: at equality the index-only haircut is undefined.
#[test]
fn apply_pro_rata_liquidity_index_haircut_errors_when_loss_meets_total_liquidity() {
    let mut r = reserve(Decimal256::one(), Decimal256::one(), 100, 0);
    let err = apply_pro_rata_liquidity_index_haircut(&mut r, 100).unwrap_err();
    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(
                message.contains("meets or exceeds total_liquidity"),
                "{}",
                message
            );
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
    assert_eq!(r.liquidity_index, Decimal256::one());
}

/// Near-total loss: factor \((L-1)/L\) is tiny but valid; index scales down without rounding up.
#[test]
fn apply_pro_rata_liquidity_index_haircut_accepts_loss_one_below_total_liquidity() {
    let mut r = reserve(Decimal256::one(), Decimal256::one(), 1_000_000, 0);
    apply_pro_rata_liquidity_index_haircut(&mut r, 999_999).unwrap();
    let exp = Decimal256::from_ratio(1u128, 1_000_000u128);
    assert_eq!(r.liquidity_index, exp);
}

/// `liquidity_index` ≠ 1: expected index still matches `I × (L − loss) / L` with [`ReserveStateV1::total_liquidity`].
#[test]
fn apply_pro_rata_liquidity_index_haircut_non_unit_index_matches_total_liquidity_formula() {
    let i0 = Decimal256::from_str("1.25").unwrap();
    let mut r = reserve(i0, Decimal256::one(), 4_000_000, 0);
    let l = r.total_liquidity().unwrap();
    let loss = 100_000u128;
    apply_pro_rata_liquidity_index_haircut(&mut r, loss).unwrap();
    let d = Decimal256::from_ratio(loss, 1u128);
    let factor = l.checked_sub(d).unwrap().checked_div(l).unwrap();
    let exp = i0.checked_mul(factor).unwrap();
    assert_eq!(r.liquidity_index, exp);
}

/// Fractional total liquidity (`scaled × index`): `loss` is compared to exact `L` in [`Decimal256`].
/// Truncating division makes `I'` slightly **below** the real 0.1 (protocol-favorable vs rounding up).
#[test]
fn apply_pro_rata_liquidity_index_haircut_with_fractional_total_liquidity() {
    let li = Decimal256::from_str("1.1").unwrap();
    let mut r = reserve(li, Decimal256::one(), 2, 0);
    // L = 2 * 1.1 = 2.2; loss 2 < 2.2 ⇒ factor = floor((L-loss)/L); I' = li * factor
    apply_pro_rata_liquidity_index_haircut(&mut r, 2).unwrap();
    let ideal = Decimal256::from_str("0.1").unwrap();
    assert!(r.liquidity_index <= ideal);
    assert_near(
        r.liquidity_index,
        ideal,
        "haircut index should be ideal 0.1 minus at most trunc dust",
    );
}
