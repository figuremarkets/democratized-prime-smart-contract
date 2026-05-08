//! Tests for UpdateSupportedCollateral execute: success (add, update, remove unused);
//! failures for non-owner, with funds, duplicate asset id, and removing asset in use.

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, ATTRIBUTE_SUPPORTED_COLLATERAL_REMOVED_JSON,
    ATTRIBUTE_SUPPORTED_COLLATERAL_UPDATED_JSON,
};
use crate::contract::execute;
use crate::execute::update_supported_collateral::{ACTION, ASSERT_OWNER_ERR};
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError;
use crate::model::{CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::get_contract_state_v1;
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{coin, Addr, Decimal256, Uint128};
use cosmwasm_std::{Env, MemoryStorage, OwnedDeps};
use provwasm_mocks::mock_provenance_dependencies;
use std::str::FromStr;

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
const BORROWER: &str = "tp1borrower";
/// Valid Provenance bech32 so addr_validate passes in instantiate.
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
const ASSET_ONE: &str = "asset.one";
const ASSET_TWO: &str = "asset.two";
const ASSET_THREE: &str = "asset.three";

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
        liquidation_bonus_rate: Decimal256::from_ratio(102u128, 100u128), // 2%
        min_lend: Uint128::new(1),
        min_borrow: Uint128::new(1),
        supported_collateral_assets: vec![
            CollateralAssetV1 {
                asset_id: ASSET_ONE.to_string(),
                haircut: Some(Decimal256::percent(80)),
            },
            CollateralAssetV1 {
                asset_id: ASSET_TWO.to_string(),
                haircut: Some(Decimal256::percent(50)),
            },
        ],
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
    let msg = default_instantiate_msg();
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .expect("instantiate should succeed");
    (deps, env)
}

#[test]
fn update_supported_collateral_succeeds_add_new_asset() {
    let (mut deps, env) = setup_instantiated();
    let to_update = vec![CollateralAssetV1 {
        asset_id: ASSET_THREE.to_string(),
        haircut: Some(Decimal256::percent(60)),
    }];
    let to_remove: Vec<String> = vec![];

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateSupportedCollateral {
            to_update: to_update.clone(),
            to_remove: to_remove.clone(),
        },
    )
    .expect("update_supported_collateral should succeed");

    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(
        res.attributes[1].key,
        ATTRIBUTE_SUPPORTED_COLLATERAL_UPDATED_JSON
    );
    let updated: Vec<String> = serde_json::from_str(&res.attributes[1].value)
        .expect("supported_collateral_updated is JSON");
    assert_eq!(updated.as_slice(), &[ASSET_THREE]);

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.supported_collateral_assets.len(), 3);
    let three = contract
        .supported_collateral_assets
        .iter()
        .find(|a| a.asset_id == ASSET_THREE)
        .unwrap();
    assert_eq!(three.haircut, Some(Decimal256::percent(60)));
}

#[test]
fn update_supported_collateral_succeeds_update_existing() {
    let (mut deps, env) = setup_instantiated();
    let to_update = vec![CollateralAssetV1 {
        asset_id: ASSET_ONE.to_string(),
        haircut: Some(Decimal256::percent(70)),
    }];
    let to_remove: Vec<String> = vec![];

    execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateSupportedCollateral {
            to_update,
            to_remove,
        },
    )
    .expect("update_supported_collateral should succeed");

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let one = contract
        .supported_collateral_assets
        .iter()
        .find(|a| a.asset_id == ASSET_ONE)
        .unwrap();
    assert_eq!(one.haircut, Some(Decimal256::percent(70)));
}

#[test]
fn update_supported_collateral_succeeds_remove_unused() {
    let (mut deps, env) = setup_instantiated();
    let to_update: Vec<CollateralAssetV1> = vec![];
    let to_remove = vec![ASSET_TWO.to_string()];

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateSupportedCollateral {
            to_update: to_update.clone(),
            to_remove: to_remove.clone(),
        },
    )
    .expect("update_supported_collateral should succeed");

    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(
        res.attributes[1].key,
        ATTRIBUTE_SUPPORTED_COLLATERAL_REMOVED_JSON
    );
    let removed: Vec<String> = serde_json::from_str(&res.attributes[1].value)
        .expect("supported_collateral_removed is JSON");
    assert_eq!(removed.as_slice(), &[ASSET_TWO]);

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.supported_collateral_assets.len(), 1);
    assert_eq!(contract.supported_collateral_assets[0].asset_id, ASSET_ONE);
}

#[test]
fn update_supported_collateral_succeeds_emits_separate_attributes() {
    let (mut deps, env) = setup_instantiated();
    let to_update = vec![
        CollateralAssetV1 {
            asset_id: ASSET_THREE.to_string(),
            haircut: Some(Decimal256::percent(60)),
        },
        CollateralAssetV1 {
            asset_id: ASSET_ONE.to_string(),
            haircut: Some(Decimal256::percent(75)),
        },
    ];
    let to_remove = vec![ASSET_TWO.to_string()];

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateSupportedCollateral {
            to_update,
            to_remove,
        },
    )
    .expect("update_supported_collateral should succeed");

    let updated_attr = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_SUPPORTED_COLLATERAL_UPDATED_JSON)
        .expect("supported_collateral_updated attribute present");
    let updated: Vec<String> =
        serde_json::from_str(&updated_attr.value).expect("supported_collateral_updated is JSON");
    let removed_attr = res
        .attributes
        .iter()
        .find(|a| a.key == ATTRIBUTE_SUPPORTED_COLLATERAL_REMOVED_JSON)
        .expect("supported_collateral_removed attribute present");
    let removed: Vec<String> =
        serde_json::from_str(&removed_attr.value).expect("supported_collateral_removed is JSON");
    assert_eq!(updated.len(), 2);
    assert!(updated.contains(&ASSET_THREE.to_string()) && updated.contains(&ASSET_ONE.to_string()));
    assert_eq!(removed.as_slice(), &[ASSET_TWO.to_string()]);
}

#[test]
fn update_supported_collateral_fails_non_owner() {
    let (mut deps, env) = setup_instantiated();
    let to_update: Vec<CollateralAssetV1> = vec![];
    let to_remove: Vec<String> = vec![];

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked("other"), &[]),
        ExecuteMsg::UpdateSupportedCollateral {
            to_update,
            to_remove,
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ContractError::NotAuthorizedError { message } if message == ASSERT_OWNER_ERR
    ));
}

#[test]
fn update_supported_collateral_fails_with_funds() {
    let (mut deps, env) = setup_instantiated();
    let to_update: Vec<CollateralAssetV1> = vec![];
    let to_remove: Vec<String> = vec![];

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[coin(100, "some.denom")]),
        ExecuteMsg::UpdateSupportedCollateral {
            to_update,
            to_remove,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::InvalidFundsError { .. } => {}
        _ => panic!("expected InvalidFundsError, got {:?}", err),
    }
}

#[test]
fn update_supported_collateral_fails_duplicate_asset_id() {
    let (mut deps, env) = setup_instantiated();
    let to_update = vec![
        CollateralAssetV1 {
            asset_id: ASSET_ONE.to_string(),
            haircut: Some(Decimal256::percent(70)),
        },
        CollateralAssetV1 {
            asset_id: ASSET_ONE.to_string(),
            haircut: Some(Decimal256::percent(80)),
        },
    ];
    let to_remove: Vec<String> = vec![];

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateSupportedCollateral {
            to_update,
            to_remove,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Duplicate asset id"));
            assert!(message.contains(ASSET_ONE));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_supported_collateral_fails_when_asset_id_is_lending_denom() {
    let (mut deps, env) = setup_instantiated();
    let lending = get_contract_state_v1(deps.as_ref().storage)
        .unwrap()
        .lending_denom
        .name
        .clone();
    let to_update = vec![CollateralAssetV1 {
        asset_id: lending.clone(),
        haircut: Some(Decimal256::percent(80)),
    }];
    let to_remove: Vec<String> = vec![];

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateSupportedCollateral {
            to_update,
            to_remove,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Collateral asset cannot be the lending denom"));
            assert!(message.contains(&lending));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_supported_collateral_fails_duplicate_when_same_id_in_update_and_remove() {
    let (mut deps, env) = setup_instantiated();
    let to_update = vec![CollateralAssetV1 {
        asset_id: ASSET_ONE.to_string(),
        haircut: Some(Decimal256::percent(70)),
    }];
    let to_remove = vec![ASSET_ONE.to_string()];

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateSupportedCollateral {
            to_update,
            to_remove,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("Duplicate asset id"),
                "expected 'Duplicate asset id' in {}",
                message
            );
            assert!(message.contains(ASSET_ONE));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn update_supported_collateral_fails_remove_asset_in_use() {
    let (mut deps, env) = setup_instantiated();
    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(BORROWER), &[coin(100, ASSET_ONE)]),
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral should succeed");

    let to_update: Vec<CollateralAssetV1> = vec![];
    let to_remove = vec![ASSET_ONE.to_string()];

    let err = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateSupportedCollateral {
            to_update,
            to_remove,
        },
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalStateError { message } => {
            assert!(message.contains("held by at least one borrower"));
            assert!(message.contains(ASSET_ONE));
        }
        _ => panic!("expected IllegalStateError, got {:?}", err),
    }
}

#[test]
fn update_supported_collateral_remove_nonexistent_is_no_op() {
    let (mut deps, env) = setup_instantiated();
    let to_update: Vec<CollateralAssetV1> = vec![];
    let to_remove = vec!["nonexistent.asset".to_string()];

    let res = execute(
        deps.as_mut(),
        env,
        message_info(&Addr::unchecked(OWNER), &[]),
        ExecuteMsg::UpdateSupportedCollateral {
            to_update,
            to_remove,
        },
    )
    .expect("update_supported_collateral should succeed");

    let contract = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(contract.supported_collateral_assets.len(), 2);
    assert_eq!(res.attributes.len(), 1);
}
