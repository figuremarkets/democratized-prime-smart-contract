//! Tests for AddCollateral execute: success with supported collateral, attributes and state;
//! failures for empty funds, unsupported denom, too many types, and missing borrower attribute.

use crate::constants::ATTRIBUTE_ACTION_NAME;
use crate::contract::execute;
use crate::execute::add_collateral::ACTION;
use crate::instantiate::instantiate_contract;
use crate::model::error::ContractError;
use crate::model::{CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::{ExecuteMsg, InstantiateMsg, RepoTokenConfig};
use crate::storage::get_borrower_collateral;
use cosmwasm_std::testing::{message_info, mock_env, MockApi};
use cosmwasm_std::{coin, Addr, Decimal256, Uint128};
use cosmwasm_std::{Env, MemoryStorage, OwnedDeps};
use provwasm_mocks::mock_provenance_dependencies;
use provwasm_std::types::provenance::attribute::v1::{
    Attribute, AttributeType, QueryAttributeRequest, QueryAttributeResponse,
};
use std::collections::BTreeMap;
use std::str::FromStr;

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
const BORROWER: &str = "tp1borrower";
/// Valid Provenance bech32 so addr_validate passes in instantiate.
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
const ASSET_ONE: &str = "asset.one";
const ASSET_TWO: &str = "asset.two";

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
fn add_collateral_succeeds_and_stores_amounts() {
    let (mut deps, env) = setup_instantiated();
    let info = message_info(&Addr::unchecked(BORROWER), &[coin(100_000_000, ASSET_ONE)]);

    let res = execute(
        deps.as_mut(),
        env.clone(),
        info,
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral should succeed");

    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].key, "borrower");
    assert_eq!(res.attributes[1].value, BORROWER);
    assert_eq!(res.attributes[2].key, "collateral_json");
    let collateral: BTreeMap<String, String> =
        serde_json::from_str(&res.attributes[2].value).expect("collateral attribute must be JSON");
    assert_eq!(
        collateral.get(ASSET_ONE).map(|s| s.as_str()),
        Some("100000000")
    );

    let collateral = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(collateral.amounts.get(ASSET_ONE), Some(&100_000_000));
}

#[test]
fn add_collateral_succeeds_multiple_denoms_and_emits_pairs() {
    let (mut deps, env) = setup_instantiated();
    let info = message_info(
        &Addr::unchecked(BORROWER),
        &[coin(100, ASSET_ONE), coin(200, ASSET_TWO)],
    );

    let res = execute(
        deps.as_mut(),
        env.clone(),
        info,
        ExecuteMsg::AddCollateral {},
    )
    .expect("add_collateral should succeed");

    assert_eq!(res.attributes[0].value, ACTION);
    assert_eq!(res.attributes[1].value, BORROWER);
    let collateral: BTreeMap<String, String> = res
        .attributes
        .iter()
        .find(|a| a.key == "collateral_json")
        .map(|a| serde_json::from_str(&a.value).expect("collateral attribute must be JSON"))
        .expect("collateral attribute present");
    assert_eq!(collateral.get(ASSET_ONE).map(|s| s.as_str()), Some("100"));
    assert_eq!(collateral.get(ASSET_TWO).map(|s| s.as_str()), Some("200"));

    let collateral = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(collateral.amounts.get(ASSET_ONE), Some(&100));
    assert_eq!(collateral.amounts.get(ASSET_TWO), Some(&200));
}

#[test]
fn add_collateral_adds_to_existing_same_denom() {
    let (mut deps, env) = setup_instantiated();
    let addr = Addr::unchecked(BORROWER);

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&addr, &[coin(100, ASSET_ONE)]),
        ExecuteMsg::AddCollateral {},
    )
    .unwrap();

    execute(
        deps.as_mut(),
        env.clone(),
        message_info(&addr, &[coin(50, ASSET_ONE)]),
        ExecuteMsg::AddCollateral {},
    )
    .unwrap();

    let collateral = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(collateral.amounts.get(ASSET_ONE), Some(&150));
}

#[test]
fn add_collateral_fails_empty_funds() {
    let (mut deps, env) = setup_instantiated();
    let info = message_info(&Addr::unchecked(BORROWER), &[]);

    let err = execute(deps.as_mut(), env, info, ExecuteMsg::AddCollateral {}).unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("At least one collateral coin must be sent"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn add_collateral_fails_unsupported_denom() {
    let (mut deps, env) = setup_instantiated();
    let info = message_info(
        &Addr::unchecked(BORROWER),
        &[coin(100, "unsupported.denom")],
    );

    let err = execute(deps.as_mut(), env, info, ExecuteMsg::AddCollateral {}).unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Unsupported collateral asset"));
            assert!(message.contains("unsupported.denom"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn add_collateral_fails_too_many_collateral_types() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();
    let mut msg = default_instantiate_msg();
    msg.max_borrower_collateral_types = 1;
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap();

    let info = message_info(
        &Addr::unchecked(BORROWER),
        &[coin(1, ASSET_ONE), coin(1, ASSET_TWO)],
    );
    let err = execute(deps.as_mut(), env, info, ExecuteMsg::AddCollateral {}).unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Too many collateral types"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn add_collateral_fails_when_borrower_attr_required_but_missing() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();
    let mut msg = default_instantiate_msg();
    msg.borrower_required_attrs = vec!["borrower.kyc".to_string()];
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap();

    let empty_attr = QueryAttributeResponse {
        account: BORROWER.to_string(),
        attributes: vec![],
        pagination: None,
    };
    QueryAttributeRequest::mock_response(&mut deps.querier, empty_attr);

    let info = message_info(&Addr::unchecked(BORROWER), &[coin(100, ASSET_ONE)]);
    let err = execute(deps.as_mut(), env, info, ExecuteMsg::AddCollateral {}).unwrap_err();

    match &err {
        ContractError::NotAuthorizedError { message } => {
            assert!(message.contains("borrower"));
            assert!(message.contains("borrower.kyc"));
        }
        _ => panic!("expected NotAuthorizedError, got {:?}", err),
    }
}

#[test]
fn add_collateral_succeeds_when_borrower_attr_required_and_present() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();
    let mut msg = default_instantiate_msg();
    msg.borrower_required_attrs = vec!["borrower.kyc".to_string()];
    instantiate_contract(
        deps.as_mut(),
        env.clone(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap();

    let attr_resp = QueryAttributeResponse {
        account: BORROWER.to_string(),
        attributes: vec![Attribute {
            name: "borrower.kyc".to_string(),
            value: b"verified".to_vec(),
            attribute_type: AttributeType::String.into(),
            address: "".to_string(),
            expiration_date: None,
            concrete_type: "".to_owned(),
        }],
        pagination: None,
    };
    QueryAttributeRequest::mock_response(&mut deps.querier, attr_resp);

    let info = message_info(&Addr::unchecked(BORROWER), &[coin(100, ASSET_ONE)]);
    let res = execute(deps.as_mut(), env, info, ExecuteMsg::AddCollateral {})
        .expect("add_collateral should succeed");

    assert_eq!(res.attributes[0].value, ACTION);
    let collateral = get_borrower_collateral(deps.as_ref().storage, BORROWER).unwrap();
    assert_eq!(collateral.amounts.get(ASSET_ONE), Some(&100));
}
