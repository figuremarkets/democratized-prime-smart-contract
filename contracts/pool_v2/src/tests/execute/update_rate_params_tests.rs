//! Tests for UpdateRateParams: success (owner updates), failures for non-owner, with funds, invalid params.

use crate::constants::ATTRIBUTE_ACTION_NAME;
use crate::contract::execute;
use crate::execute::update_rate_params::{ACTION, ASSERT_OWNER_ERR};
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError;
use crate::model::{CollateralAssetV1, Denom, FeeModelV1, RateParamsV1};
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::{get_contract_state_v1, get_reserve_state_v1};
use crate::tests::response_attrs::assert_response_lend_borrow_rates_match_reserve;
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{coin, Addr, Decimal256, Timestamp, Uint128};
use cosmwasm_std::{Env, MemoryStorage, OwnedDeps};
use provwasm_mocks::mock_provenance_dependencies;
use std::str::FromStr;

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";

fn default_instantiate_msg() -> InstantiateMsg {
    InstantiateMsg {
        contract_name: "pool-v2-demo".to_string(),
        description: "Test pool v2".to_string(),
        repo_token: RepoTokenConfig::Existing {
            repo_token_cw20_contract_address: REPO_TOKEN_CW20.to_string(),
        },
        lending_denom: Denom::new("uylds.fcc", 6u32),
        rate_params: RateParamsV1 {
            target_rate: Decimal256::from_str("0.09").unwrap(),
            min_rate: Decimal256::from_str("0.0325").unwrap(),
            max_rate: Decimal256::from_str("0.20").unwrap(),
            kink_utilization: Decimal256::from_str("0.90").unwrap(),
            reserve_factor: Decimal256::from_str("0.005").unwrap(),
            fee_model: Default::default(),
            flat_fee_apr: Decimal256::zero(),
            seconds_per_year: 31_536_000,
        },
        lender_required_attrs: vec![],
        borrower_required_attrs: vec![],
        price_oracle_address: ORACLE.to_string(),
        max_borrower_collateral_types: 5,
        margin_rate: Decimal256::from_str("0.80").unwrap(),
        liquidation_rate: Decimal256::from_str("0.90").unwrap(),
        liquidation_bonus_rate: Decimal256::from_ratio(102u128, 100u128),
        min_lend: Uint128::new(1),
        min_borrow: Uint128::new(1),
        supported_collateral_assets: vec![CollateralAssetV1 {
            asset_id: "asset.one".to_string(),
            haircut: Some(Decimal256::percent(80)),
        }],
        commit_market_id: None,
        bad_debt_loss_allocation: Default::default(),
    }
}

fn setup_instantiated() -> (
    OwnedDeps<MemoryStorage, MockApi, provwasm_mocks::MockProvenanceQuerier>,
    Env,
) {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        default_instantiate_msg(),
    )
    .expect("instantiate should succeed");
    (deps, env)
}

#[test]
fn update_rate_params_succeeds() {
    let (mut deps, env) = setup_instantiated();

    let new_params = RateParamsV1 {
        target_rate: Decimal256::from_str("0.10").unwrap(),
        min_rate: Decimal256::from_str("0.04").unwrap(),
        max_rate: Decimal256::from_str("0.22").unwrap(),
        kink_utilization: Decimal256::from_str("0.85").unwrap(),
        reserve_factor: Decimal256::from_str("0.01").unwrap(),
        fee_model: Default::default(),
        flat_fee_apr: Decimal256::zero(),
        seconds_per_year: 31_536_000,
    };

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateRateParams {
            rate_params: new_params.clone(),
        },
    )
    .expect("update_rate_params should succeed");

    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, ACTION);

    assert_response_lend_borrow_rates_match_reserve(&res, deps.as_ref().storage);

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.rate_params.target_rate, new_params.target_rate);
    assert_eq!(contract.rate_params.min_rate, new_params.min_rate);
    assert_eq!(contract.rate_params.max_rate, new_params.max_rate);
    assert_eq!(
        contract.rate_params.kink_utilization,
        new_params.kink_utilization
    );
    assert_eq!(
        contract.rate_params.reserve_factor,
        new_params.reserve_factor
    );
}

#[test]
fn update_rate_params_accrues_reserve_to_current_block() {
    let (mut deps, mut env) = setup_instantiated();
    let reserve_before = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    let t_before = reserve_before.last_updated_at.seconds();
    env.block.time = Timestamp::from_seconds(t_before + 31_536_000); // +1 year

    let new_params = RateParamsV1 {
        target_rate: Decimal256::from_str("0.10").unwrap(),
        min_rate: Decimal256::from_str("0.04").unwrap(),
        max_rate: Decimal256::from_str("0.22").unwrap(),
        kink_utilization: Decimal256::from_str("0.85").unwrap(),
        reserve_factor: Decimal256::from_str("0.01").unwrap(),
        fee_model: Default::default(),
        flat_fee_apr: Decimal256::zero(),
        seconds_per_year: 31_536_000,
    };

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateRateParams {
            rate_params: new_params,
        },
    )
    .expect("update_rate_params should succeed");

    let reserve_after = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(
        reserve_after.last_updated_at, env.block.time,
        "reserve must be accrued to current block so new rate params apply from now"
    );
}

#[test]
fn update_rate_params_fails_non_owner() {
    let (mut deps, env) = setup_instantiated();

    let new_params = RateParamsV1 {
        target_rate: Decimal256::from_str("0.10").unwrap(),
        min_rate: Decimal256::from_str("0.04").unwrap(),
        max_rate: Decimal256::from_str("0.22").unwrap(),
        kink_utilization: Decimal256::from_str("0.85").unwrap(),
        reserve_factor: Decimal256::from_str("0.01").unwrap(),
        fee_model: Default::default(),
        flat_fee_apr: Decimal256::zero(),
        seconds_per_year: 31_536_000,
    };

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked("other"), &[]),
        ExecuteMsg::UpdateRateParams {
            rate_params: new_params,
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_ERR
    ));
}

#[test]
fn update_rate_params_fails_with_funds() {
    let (mut deps, env) = setup_instantiated();

    let new_params = RateParamsV1 {
        target_rate: Decimal256::from_str("0.10").unwrap(),
        min_rate: Decimal256::from_str("0.04").unwrap(),
        max_rate: Decimal256::from_str("0.22").unwrap(),
        kink_utilization: Decimal256::from_str("0.85").unwrap(),
        reserve_factor: Decimal256::from_str("0.01").unwrap(),
        fee_model: Default::default(),
        flat_fee_apr: Decimal256::zero(),
        seconds_per_year: 31_536_000,
    };

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, "uylds.fcc")]),
        ExecuteMsg::UpdateRateParams {
            rate_params: new_params,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::InvalidFundsError { message } => {
            assert!(message.contains("No funds accepted"));
        }
        _ => panic!("expected InvalidFundsError, got {:?}", err),
    }
}

#[test]
fn update_rate_params_fails_invalid_min_gt_target() {
    let (mut deps, env) = setup_instantiated();

    let invalid_params = RateParamsV1 {
        target_rate: Decimal256::from_str("0.05").unwrap(),
        min_rate: Decimal256::from_str("0.10").unwrap(), // min > target
        max_rate: Decimal256::from_str("0.20").unwrap(),
        kink_utilization: Decimal256::from_str("0.90").unwrap(),
        reserve_factor: Decimal256::from_str("0.005").unwrap(),
        fee_model: Default::default(),
        flat_fee_apr: Decimal256::zero(),
        seconds_per_year: 31_536_000,
    };

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateRateParams {
            rate_params: invalid_params,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("rate_params"));
            assert!(message.contains("min_rate"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_rate_params_fails_reserve_factor_one() {
    let (mut deps, env) = setup_instantiated();

    let invalid_params = RateParamsV1 {
        target_rate: Decimal256::from_str("0.09").unwrap(),
        min_rate: Decimal256::from_str("0.0325").unwrap(),
        max_rate: Decimal256::from_str("0.20").unwrap(),
        kink_utilization: Decimal256::from_str("0.90").unwrap(),
        reserve_factor: Decimal256::one(),
        fee_model: Default::default(),
        flat_fee_apr: Decimal256::zero(),
        seconds_per_year: 31_536_000,
    };

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateRateParams {
            rate_params: invalid_params,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("reserve_factor"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_rate_params_fails_invalid_kink_zero() {
    let (mut deps, env) = setup_instantiated();

    let invalid_params = RateParamsV1 {
        target_rate: Decimal256::from_str("0.09").unwrap(),
        min_rate: Decimal256::from_str("0.0325").unwrap(),
        max_rate: Decimal256::from_str("0.20").unwrap(),
        kink_utilization: Decimal256::zero(),
        reserve_factor: Decimal256::from_str("0.005").unwrap(),
        fee_model: Default::default(),
        flat_fee_apr: Decimal256::zero(),
        seconds_per_year: 31_536_000,
    };

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateRateParams {
            rate_params: invalid_params,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("kink_utilization"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_rate_params_succeeds_flat_borrow_spread_mode() {
    let (mut deps, env) = setup_instantiated();
    let new_params = RateParamsV1 {
        target_rate: Decimal256::from_str("0.10").unwrap(),
        min_rate: Decimal256::from_str("0.04").unwrap(),
        max_rate: Decimal256::from_str("0.22").unwrap(),
        kink_utilization: Decimal256::from_str("0.85").unwrap(),
        reserve_factor: Decimal256::from_str("0.01").unwrap(),
        fee_model: FeeModelV1::FlatBorrowSpread,
        flat_fee_apr: Decimal256::from_str("0.005").unwrap(),
        seconds_per_year: 31_536_000,
    };

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateRateParams {
            rate_params: new_params.clone(),
        },
    )
    .expect("flat spread update should succeed");

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.rate_params.fee_model, FeeModelV1::FlatBorrowSpread);
    assert_eq!(contract.rate_params.flat_fee_apr, new_params.flat_fee_apr);
}

#[test]
fn update_rate_params_fails_reserve_factor_with_non_zero_flat_fee() {
    let (mut deps, env) = setup_instantiated();
    let invalid_params = RateParamsV1 {
        target_rate: Decimal256::from_str("0.10").unwrap(),
        min_rate: Decimal256::from_str("0.04").unwrap(),
        max_rate: Decimal256::from_str("0.22").unwrap(),
        kink_utilization: Decimal256::from_str("0.85").unwrap(),
        reserve_factor: Decimal256::from_str("0.01").unwrap(),
        fee_model: FeeModelV1::ReserveFactor,
        flat_fee_apr: Decimal256::from_str("0.001").unwrap(),
        seconds_per_year: 31_536_000,
    };

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateRateParams {
            rate_params: invalid_params,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("flat_fee_apr"));
            assert!(message.contains("reserve_factor"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}
