//! Unit tests for pool_v2 utils/math.rs

use crate::utils::{checked_div_ceil, format_as_percent_string, uint128_to_decimal256};
use cosmwasm_std::{Decimal256, Uint128};
use std::str::FromStr;

#[test]
fn uint128_to_decimal256_zero() {
    assert_eq!(uint128_to_decimal256(0u128), Decimal256::zero());
}

#[test]
fn uint128_to_decimal256_one() {
    assert_eq!(uint128_to_decimal256(1u128), Decimal256::one());
}

#[test]
fn uint128_to_decimal256_large() {
    let d = uint128_to_decimal256(1_000_000_000u128);
    assert_eq!(d, Decimal256::from_ratio(1_000_000_000u128, 1u128));
}

#[test]
fn uint128_to_decimal256_accepts_uint128() {
    let d = uint128_to_decimal256(Uint128::new(100));
    assert_eq!(d, Decimal256::from_ratio(100u128, 1u128));
}

fn dec(s: &str) -> Decimal256 {
    Decimal256::from_str(s).unwrap()
}

#[test]
fn checked_div_ceil_is_exact_when_quotient_fits() {
    assert_eq!(
        checked_div_ceil(dec("102"), dec("0.8")).unwrap(),
        dec("127.5")
    );
    assert_eq!(
        checked_div_ceil(Decimal256::zero(), dec("0.7")).unwrap(),
        Decimal256::zero()
    );
}

#[test]
fn checked_div_ceil_rounds_up_one_atomic_when_inexact() {
    let debt = dec("0.000001");
    let margin = dec("0.7");
    assert_eq!(
        debt.checked_div(margin).unwrap(),
        dec("0.000001428571428571")
    );
    assert_eq!(
        checked_div_ceil(debt, margin).unwrap(),
        dec("0.000001428571428572")
    );
}

/// Small debt at a non-terminating margin rate: collateral equal to the truncated requirement
/// puts LTV above margin_rate; the ceiled requirement keeps it at or below.
#[test]
fn checked_div_ceil_requirement_keeps_ltv_within_margin() {
    let debt = dec("0.000001");
    let margin = dec("0.7");
    let truncated = debt.checked_div(margin).unwrap();
    assert!(debt.checked_div(truncated).unwrap() > margin);
    let ceiled = checked_div_ceil(debt, margin).unwrap();
    assert!(debt.checked_div(ceiled).unwrap() <= margin);
}

#[test]
fn checked_div_ceil_rejects_zero_denominator() {
    assert!(checked_div_ceil(Decimal256::one(), Decimal256::zero()).is_err());
}

#[test]
fn format_as_percent_string_zero() {
    let s = format_as_percent_string(Decimal256::zero()).expect("ok");
    assert_eq!(s, "0%");
}

#[test]
fn format_as_percent_string_one_hundred() {
    let s = format_as_percent_string(Decimal256::one()).expect("ok");
    assert_eq!(s, "100%");
}

#[test]
fn format_as_percent_string_half() {
    let half = Decimal256::from_str("0.5").unwrap();
    let s = format_as_percent_string(half).expect("ok");
    assert_eq!(s, "50%");
}

#[test]
fn format_as_percent_string_margin_rate_style() {
    let rate = Decimal256::from_str("0.80").unwrap();
    let s = format_as_percent_string(rate).expect("ok");
    assert!(s.starts_with("80"));
    assert!(s.ends_with('%'));
}
