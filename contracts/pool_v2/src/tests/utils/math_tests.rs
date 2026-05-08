//! Unit tests for pool_v2 utils/math.rs

use crate::utils::{format_as_percent_string, uint128_to_decimal256};
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
