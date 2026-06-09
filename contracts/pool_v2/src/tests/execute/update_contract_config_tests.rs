//! Tests for UpdateContractConfig: success (single and multiple fields), failures for non-owner,
//! with funds, no fields, and invalid rate combinations.

use crate::constants::{ATTRIBUTE_ACTION_NAME, ATTRIBUTE_CONTRACT_STATE_JSON};
use crate::contract::execute;
use crate::execute::update_contract_config::{ACTION, ASSERT_OWNER_ERR};
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError;
use crate::model::{BadDebtLossAllocation, CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::{get_contract_state_v1, get_reserve_state_v1, set_reserve_state_v1};
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{coin, Addr, Decimal256, Response, Uint128};
use cosmwasm_std::{Env, MemoryStorage, OwnedDeps};
use provwasm_mocks::mock_provenance_dependencies;
use std::str::FromStr;

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
/// Another valid Provenance bech32 address (from withdraw_reserve_tests).
const ORACLE_ALT: &str = "tp1tkn2dwfkx7pmjr2rtgqhtrudsv7h8w2tj6eesv";

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
fn update_contract_config_succeeds_single_field() {
    let (mut deps, env) = setup_instantiated();

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: Some(Uint128::new(100)),
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("update_contract_config should succeed");

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.min_lend, Uint128::new(100));
    assert_eq!(contract.min_borrow, Uint128::new(1));
    assert_eq!(contract.margin_rate, Decimal256::from_str("0.80").unwrap());
}

// --- bad_debt_loss_allocation (deficit must be zero to change mode) ---

#[test]
fn update_contract_config_sets_bad_debt_loss_allocation() {
    let (mut deps, env) = setup_instantiated();
    let c0 = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(
        c0.bad_debt_loss_allocation,
        BadDebtLossAllocation::DeferredToDeficit
    );

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Some(BadDebtLossAllocation::ImmediateLiquidityIndexHaircut),
        },
    )
    .expect("update bad_debt_loss_allocation");

    let c = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(
        c.bad_debt_loss_allocation,
        BadDebtLossAllocation::ImmediateLiquidityIndexHaircut
    );
}

#[test]
fn update_contract_config_rejects_bad_debt_allocation_change_when_deficit_positive() {
    let (mut deps, env) = setup_instantiated();
    let mut r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    r.deficit_underlying = 1;
    set_reserve_state_v1(deps.as_mut().storage, &r).unwrap();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Some(BadDebtLossAllocation::ImmediateLiquidityIndexHaircut),
        },
    )
    .unwrap_err();
    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("bad_debt_loss_allocation")
                    && message.contains("deficit_underlying"),
                "{}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_contract_config_allows_other_fields_when_deficit_positive() {
    let (mut deps, env) = setup_instantiated();
    let mut r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    r.deficit_underlying = 1;
    set_reserve_state_v1(deps.as_mut().storage, &r).unwrap();

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: Some(Decimal256::from_str("0.79").unwrap()),
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: None,
        },
    )
    .expect("margin_rate update with deficit");

    let c = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(c.margin_rate, Decimal256::from_str("0.79").unwrap());
}

#[test]
fn update_contract_config_allows_redundant_bad_debt_allocation_when_deficit_positive() {
    let (mut deps, env) = setup_instantiated();
    let mut r = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    r.deficit_underlying = 1;
    set_reserve_state_v1(deps.as_mut().storage, &r).unwrap();

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Some(BadDebtLossAllocation::DeferredToDeficit),
        },
    )
    .expect("no-op bad_debt_loss_allocation with deficit");
}

// --- Other UpdateContractConfig ---

#[test]
fn update_contract_config_succeeds_multiple_fields() {
    let (mut deps, env) = setup_instantiated();

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: Some(Decimal256::from_str("0.75").unwrap()),
            liquidation_rate: Some(Decimal256::from_str("0.92").unwrap()),
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: Some(Uint128::new(50)),
            max_borrower_collateral_types: Some(3),
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("update_contract_config should succeed");

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.margin_rate, Decimal256::from_str("0.75").unwrap());
    assert_eq!(
        contract.liquidation_rate,
        Decimal256::from_str("0.92").unwrap()
    );
    assert_eq!(contract.min_borrow, Uint128::new(50));
    assert_eq!(contract.max_borrower_collateral_types, 3);
}

#[test]
fn update_contract_config_succeeds_price_oracle() {
    let (mut deps, env) = setup_instantiated();

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: Some(ORACLE_ALT.to_string()),
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("update_contract_config should succeed");

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.price_oracle_address.as_str(), ORACLE_ALT);
}

#[test]
fn update_contract_config_emits_action() {
    let (mut deps, env) = setup_instantiated();

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: Some(Uint128::new(10)),
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("update_contract_config should succeed");

    let result_contract_state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(
        res,
        Response::new()
            .add_attribute(ATTRIBUTE_ACTION_NAME, ACTION)
            .add_attribute(
                ATTRIBUTE_CONTRACT_STATE_JSON,
                serde_json::to_string(&result_contract_state).unwrap()
            )
    )
}

#[test]
fn update_contract_config_fails_non_owner() {
    let (mut deps, env) = setup_instantiated();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked("other"), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: Some(Uint128::new(10)),
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_ERR
    ));
}

#[test]
fn update_contract_config_fails_with_funds() {
    let (mut deps, env) = setup_instantiated();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(1, "uylds.fcc")]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: Some(Uint128::new(10)),
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::InvalidFundsError { .. } => {}
        _ => panic!("expected InvalidFundsError, got {:?}", err),
    }
}

#[test]
fn update_contract_config_fails_no_fields() {
    let (mut deps, env) = setup_instantiated();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("At least one config field must be provided"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_contract_config_fails_margin_not_less_than_liquidation() {
    let (mut deps, env) = setup_instantiated();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: Some(Decimal256::from_str("0.90").unwrap()),
            liquidation_rate: Some(Decimal256::from_str("0.90").unwrap()),
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("margin_rate must be less than liquidation_rate"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_contract_config_fails_when_liquidation_rate_is_greater_than_one() {
    let (mut deps, env) = setup_instantiated();
    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: Some(Decimal256::from_str("1.000001").unwrap()),
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("liquidation_rate must be less than or equal to 1"),
                "expected message about liquidation_rate increase, got: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_contract_config_fails_liquidation_rate_decrease() {
    let (mut deps, env) = setup_instantiated();
    // Default liquidation_rate is 0.90; decreasing would make more positions liquidatable.

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: Some(Decimal256::from_str("0.88").unwrap()),
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("liquidation_rate may only be increased"),
                "expected message about liquidation_rate increase, got: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_contract_config_succeeds_liquidation_rate_increase() {
    let (mut deps, env) = setup_instantiated();

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: Some(Decimal256::from_str("0.95").unwrap()),
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("increasing liquidation_rate should succeed");

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(
        contract.liquidation_rate,
        Decimal256::from_str("0.95").unwrap()
    );
}

#[test]
fn update_contract_config_succeeds_commit_market_id_set() {
    let (mut deps, env) = setup_instantiated();

    assert!(get_contract_state_v1(deps.as_ref().storage)
        .unwrap()
        .commit_market_id
        .is_none());

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("set commit_market_id should succeed");

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.commit_market_id, Some(1));
}

/// `commit_market_id` is only updated when the field is included with an integer; other config
/// patches must not clear or overwrite it.
#[test]
fn update_contract_config_preserves_commit_market_id_when_not_patched() {
    let (mut deps, env) = setup_instantiated();

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: Some(1),
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("set commit_market_id should succeed");

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: Some(Uint128::new(99)),
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .expect("patch min_lend only should succeed");

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.commit_market_id, Some(1));
    assert_eq!(contract.min_lend, Uint128::new(99));
}

#[test]
fn update_contract_config_fails_bonus_not_gt_one() {
    let (mut deps, env) = setup_instantiated();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: Some(Decimal256::one()),
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("liquidation_bonus_rate must be greater than 1"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_contract_config_fails_min_lend_zero() {
    let (mut deps, env) = setup_instantiated();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: Some(Uint128::zero()),
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("min_lend must be at least 1"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_contract_config_fails_max_borrower_collateral_types_zero() {
    let (mut deps, env) = setup_instantiated();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: None,
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: Some(0),
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("max_borrower_collateral_types must be at least 1"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_contract_config_fails_empty_price_oracle() {
    let (mut deps, env) = setup_instantiated();

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateContractConfig {
            margin_rate: None,
            liquidation_rate: None,
            liquidation_bonus_rate: None,
            price_oracle_address: Some("   ".to_string()),
            min_lend: None,
            min_borrow: None,
            max_borrower_collateral_types: None,
            commit_market_id: None,
            bad_debt_loss_allocation: Default::default(),
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("price_oracle_address cannot be empty"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}
