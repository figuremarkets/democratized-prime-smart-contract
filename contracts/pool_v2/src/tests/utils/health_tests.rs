//! Unit tests for pool_v2 utils/health.rs: LTV, health states, collateral/debt value, validate_borrower_is_healthy.

use crate::model::collateral::{BorrowerCollateralV1, CollateralAssetV1};
use crate::model::contract_state::ContractStateV1;
use crate::model::error::ContractError;
use crate::model::health::BorrowerHealthV1;
use crate::model::{Denom, OperationalState, RateParamsV1};
use crate::utils::{
    calculate_borrow_value_usd, calculate_ltv, calculate_total_collateral_value_usd,
    get_health_from_ltv, validate_borrower_is_healthy,
};
use cosmwasm_std::{Addr, Decimal256, Uint128};
use democratized_prime_lib::price_oracle::model::{AssetPriceResponseV1, PriceMapResponse};
use std::str::FromStr;

fn contract_state(margin_rate: &str, liquidation_rate: &str) -> ContractStateV1 {
    ContractStateV1 {
        contract_name: "test".to_string(),
        description: "".to_string(),
        repo_token_cw20_address: Some(Addr::unchecked("repo_cw20")),
        lending_denom: Denom::new("lend", 6u32),
        rate_params: RateParamsV1 {
            target_rate: Decimal256::from_str("0.06").unwrap(),
            min_rate: Decimal256::from_str("0.01").unwrap(),
            max_rate: Decimal256::from_str("0.50").unwrap(),
            kink_utilization: Decimal256::from_str("0.80").unwrap(),
            reserve_factor: Decimal256::from_str("0.10").unwrap(),
            seconds_per_year: 31_536_000,
        },
        lender_required_attrs: vec![],
        borrower_required_attrs: vec![],
        price_oracle_address: Addr::unchecked("oracle"),
        max_borrower_collateral_types: 5,
        margin_rate: Decimal256::from_str(margin_rate).unwrap(),
        liquidation_rate: Decimal256::from_str(liquidation_rate).unwrap(),
        liquidation_bonus_rate: Decimal256::from_ratio(102u128, 100u128), // 2%
        min_lend: Uint128::new(1),
        min_borrow: Uint128::new(1),
        supported_collateral_assets: vec![],
        operational_state: OperationalState::Active,
        commit_market_id: None,
        bad_debt_loss_allocation: Default::default(),
    }
}

fn contract_state_with_assets(
    margin_rate: &str,
    liquidation_rate: &str,
    assets: &[(&str, Option<&str>)],
) -> ContractStateV1 {
    let mut state = contract_state(margin_rate, liquidation_rate);
    state.supported_collateral_assets = supported_assets(assets);
    state
}

fn supported_assets(assets: &[(&str, Option<&str>)]) -> Vec<CollateralAssetV1> {
    assets
        .iter()
        .map(|(id, h)| CollateralAssetV1 {
            asset_id: (*id).to_string(),
            haircut: h.map(|s| Decimal256::from_str(s).unwrap()),
        })
        .collect()
}

fn price_entry(price_usd: &str) -> AssetPriceResponseV1 {
    AssetPriceResponseV1 {
        price_usd: Decimal256::from_str(price_usd).unwrap(),
        as_of_epoch_second: 0,
        expiration_epoch_seconds: u64::MAX,
    }
}

// ---- get_health_from_ltv ----

#[test]
fn health_from_ltv_healthy_below_margin() {
    let state = contract_state("0.80", "0.90");
    let health = get_health_from_ltv(&state, Decimal256::from_str("0.50").unwrap()).unwrap();
    assert_eq!(health, BorrowerHealthV1::Healthy);
}

#[test]
fn health_from_ltv_healthy_at_zero() {
    let state = contract_state("0.80", "0.90");
    let health = get_health_from_ltv(&state, Decimal256::zero()).unwrap();
    assert_eq!(health, BorrowerHealthV1::Healthy);
}

#[test]
fn health_from_ltv_healthy_at_margin() {
    // LTV == margin_rate is Healthy (Unhealthy only when LTV > margin_rate).
    let state = contract_state("0.80", "0.90");
    let health = get_health_from_ltv(&state, Decimal256::from_str("0.80").unwrap()).unwrap();
    assert_eq!(health, BorrowerHealthV1::Healthy);
}

#[test]
fn health_from_ltv_unhealthy_between_margin_and_liquidation() {
    let state = contract_state("0.80", "0.90");
    let health = get_health_from_ltv(&state, Decimal256::from_str("0.85").unwrap()).unwrap();
    assert_eq!(health, BorrowerHealthV1::Unhealthy);
}

#[test]
fn health_from_ltv_liquidatable_at_liquidation_rate() {
    let state = contract_state("0.80", "0.90");
    let health = get_health_from_ltv(&state, Decimal256::from_str("0.90").unwrap()).unwrap();
    assert_eq!(health, BorrowerHealthV1::Liquidatable);
}

#[test]
fn health_from_ltv_liquidatable_above_liquidation_rate() {
    let state = contract_state("0.80", "0.90");
    let health = get_health_from_ltv(&state, Decimal256::from_str("0.95").unwrap()).unwrap();
    assert_eq!(health, BorrowerHealthV1::Liquidatable);
}

// ---- calculate_borrow_value_usd ----

#[test]
fn borrow_value_usd_zero_debt() {
    let mut prices = PriceMapResponse::new();
    prices.insert("lend".to_string(), price_entry("1.0"));
    let v = calculate_borrow_value_usd(Uint128::zero(), "lend", &prices).unwrap();
    assert_eq!(v, Decimal256::zero());
}

#[test]
fn borrow_value_usd_with_price() {
    let mut prices = PriceMapResponse::new();
    prices.insert("lend".to_string(), price_entry("2.5"));
    let v = calculate_borrow_value_usd(Uint128::new(100), "lend", &prices).unwrap();
    assert_eq!(v, Decimal256::from_str("250").unwrap());
}

#[test]
fn borrow_value_usd_missing_price_errors() {
    let prices = PriceMapResponse::new();
    let err = calculate_borrow_value_usd(Uint128::new(100), "lend", &prices).unwrap_err();
    match &err {
        ContractError::NotFoundError { message } => {
            assert!(message.contains("Price of asset"));
        }
        _ => panic!("expected NotFoundError, got {:?}", err),
    }
}

// ---- calculate_total_collateral_value_usd ----

#[test]
fn total_collateral_value_empty() {
    let collateral = BorrowerCollateralV1::default();
    let prices = PriceMapResponse::new();
    let assets = supported_assets(&[]);
    let v = calculate_total_collateral_value_usd(&collateral, &prices, &assets).unwrap();
    assert_eq!(v, Decimal256::zero());
}

#[test]
fn total_collateral_value_one_asset_full_haircut() {
    let mut collateral = BorrowerCollateralV1::default();
    collateral.amounts.insert("btc".to_string(), 10u128);
    let mut prices = PriceMapResponse::new();
    prices.insert("btc".to_string(), price_entry("50000"));
    let assets = supported_assets(&[("btc", Some("1.0"))]);
    let v = calculate_total_collateral_value_usd(&collateral, &prices, &assets).unwrap();
    assert_eq!(v, Decimal256::from_str("500000").unwrap()); // 10 * 50000
}

#[test]
fn total_collateral_value_one_asset_with_haircut() {
    let mut collateral = BorrowerCollateralV1::default();
    collateral.amounts.insert("btc".to_string(), 10u128);
    let mut prices = PriceMapResponse::new();
    prices.insert("btc".to_string(), price_entry("50000"));
    let assets = supported_assets(&[("btc", Some("0.80"))]); // 80% haircut
    let v = calculate_total_collateral_value_usd(&collateral, &prices, &assets).unwrap();
    assert_eq!(v, Decimal256::from_str("400000").unwrap()); // 10 * 50000 * 0.8
}

#[test]
fn total_collateral_value_one_asset_no_haircut_set_uses_full_value() {
    // When haircut is None we treat as 100% (no discount); value = price × amount.
    let mut collateral = BorrowerCollateralV1::default();
    collateral.amounts.insert("btc".to_string(), 10u128);
    let mut prices = PriceMapResponse::new();
    prices.insert("btc".to_string(), price_entry("50000"));
    let assets = supported_assets(&[("btc", None)]); // no haircut
    let v = calculate_total_collateral_value_usd(&collateral, &prices, &assets).unwrap();
    assert_eq!(v, Decimal256::from_str("500000").unwrap()); // 10 * 50000 * 1.0
}

#[test]
fn total_collateral_value_missing_price_errors() {
    let mut collateral = BorrowerCollateralV1::default();
    collateral.amounts.insert("btc".to_string(), 10u128);
    let prices = PriceMapResponse::new();
    let assets = supported_assets(&[("btc", Some("1.0"))]);
    let err = calculate_total_collateral_value_usd(&collateral, &prices, &assets).unwrap_err();
    match &err {
        ContractError::NotFoundError { message } => {
            assert!(message.contains("Price of asset"));
        }
        _ => panic!("expected NotFoundError, got {:?}", err),
    }
}

// ---- calculate_ltv ----

#[test]
fn ltv_zero_debt_zero_collateral() {
    let state = contract_state_with_assets("0.80", "0.90", &[("btc", Some("1.0"))]);
    let collateral = BorrowerCollateralV1::default();
    let mut prices = PriceMapResponse::new();
    prices.insert("lend".to_string(), price_entry("1.0"));
    let ltv = calculate_ltv(
        &state,
        &state.supported_collateral_assets,
        &prices,
        &collateral,
        Uint128::zero(),
    )
    .unwrap();
    assert_eq!(ltv, Decimal256::zero());
}

#[test]
fn ltv_no_collateral_with_debt_errors() {
    let state = contract_state("0.80", "0.90");
    let collateral = BorrowerCollateralV1::default();
    let mut prices = PriceMapResponse::new();
    prices.insert("lend".to_string(), price_entry("1.0"));
    let err = calculate_ltv(
        &state,
        &state.supported_collateral_assets,
        &prices,
        &collateral,
        Uint128::new(100),
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("No collateral for loans"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn ltv_ratio_correct() {
    let state = contract_state_with_assets("0.80", "0.90", &[("btc", Some("1.0"))]);
    let mut collateral = BorrowerCollateralV1::default();
    collateral.amounts.insert("btc".to_string(), 10u128); // 10 * 100 = 1000 usd
    let mut prices = PriceMapResponse::new();
    prices.insert("lend".to_string(), price_entry("1.0"));
    prices.insert("btc".to_string(), price_entry("100"));
    let ltv = calculate_ltv(
        &state,
        &state.supported_collateral_assets,
        &prices,
        &collateral,
        Uint128::new(500),
    )
    .unwrap();
    // debt usd = 500 * 1 = 500, collateral usd = 1000, ltv = 0.5
    assert_eq!(ltv, Decimal256::from_str("0.5").unwrap());
}

// ---- validate_borrower_is_healthy ----

#[test]
fn validate_borrower_is_healthy_ok_when_healthy() {
    let state = contract_state("0.80", "0.90");
    assert!(validate_borrower_is_healthy(
        BorrowerHealthV1::Healthy,
        Decimal256::from_str("0.50").unwrap(),
        &state
    )
    .is_ok());
}

#[test]
fn validate_borrower_is_healthy_err_when_unhealthy() {
    let state = contract_state("0.80", "0.90");
    let err = validate_borrower_is_healthy(
        BorrowerHealthV1::Unhealthy,
        Decimal256::from_str("0.85").unwrap(),
        &state,
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Loan-to-value"));
            assert!(message.contains("Unhealthy"));
            assert!(message.contains("80")); // margin rate as percent
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn validate_borrower_is_healthy_err_when_liquidatable() {
    let state = contract_state("0.80", "0.90");
    let err = validate_borrower_is_healthy(
        BorrowerHealthV1::Liquidatable,
        Decimal256::from_str("0.95").unwrap(),
        &state,
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Liquidatable"));
            assert!(message.contains("90")); // liquidation rate as percent
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}
