//! Unit tests for pool_v2 utils/custodian.rs.

use crate::model::contract_state::ContractStateV1;
use crate::model::error::ContractError;
use crate::model::{Denom, OperationalState, RateParamsV1};
use crate::tests::query::common::{CUSTODIAN, SOME_USER};
use crate::utils::assert_custodian;
use cosmwasm_std::{Addr, Decimal256, Uint128};
use std::str::FromStr;

fn contract_state_with_custodian(custodian: Option<&str>) -> ContractStateV1 {
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
        margin_rate: Decimal256::from_str("0.80").unwrap(),
        liquidation_rate: Decimal256::from_str("0.90").unwrap(),
        liquidation_bonus_rate: Decimal256::from_ratio(102u128, 100u128),
        min_lend: Uint128::new(1),
        min_borrow: Uint128::new(1),
        supported_collateral_assets: vec![],
        operational_state: OperationalState::Active,
        commit_market_id: None,
        bad_debt_loss_allocation: Default::default(),
        custodian: custodian.map(Addr::unchecked),
    }
}

#[test]
fn assert_custodian_succeeds_for_custodian_sender() {
    let state = contract_state_with_custodian(Some(CUSTODIAN));
    assert_custodian(
        &state,
        &Addr::unchecked(CUSTODIAN),
        "only custodian allowed",
    )
    .expect("custodian sender should pass");
}

#[test]
fn assert_custodian_fails_for_non_custodian_sender() {
    let state = contract_state_with_custodian(Some(CUSTODIAN));
    let err = assert_custodian(
        &state,
        &Addr::unchecked(SOME_USER),
        "only custodian allowed",
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == "only custodian allowed"
    ));
}

#[test]
fn assert_custodian_fails_when_custodian_unset() {
    let state = contract_state_with_custodian(None);
    let err = assert_custodian(
        &state,
        &Addr::unchecked(CUSTODIAN),
        "only custodian allowed",
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == "contract custodian not set"
    ));
}
