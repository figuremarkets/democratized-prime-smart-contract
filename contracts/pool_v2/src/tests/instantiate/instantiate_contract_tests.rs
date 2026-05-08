//! Tests for pool_v2 instantiation: validation, marker create message when marker does not exist,
//! existing marker path (no create message), and state/cw2 version storage.

use crate::constants::{
    ATTRIBUTE_ACTION_NAME, CONTRACT_NAME, CONTRACT_VERSION, MAX_LENDER_BORROWER_REQUIRED_ATTRS,
};
use crate::instantiate::{instantiate_contract, reply};
use crate::model::error::ContractError;
use crate::model::{CollateralAssetV1, Denom, RateParamsV1};
use crate::msg::instantiate::{InstantiateMsg, RepoTokenConfig};
use crate::storage::{get_contract_state_v1, get_reserve_state_v1};
use crate::tests::instantiate_helpers::mock_repo_token_instantiate_reply;
use cosmwasm_std::testing::{message_info, mock_env};
use cosmwasm_std::{Addr, CosmosMsg, Decimal256, Uint128, WasmMsg};
use cw2::get_contract_version;
use cw_ownable::get_ownership;
use provwasm_mocks::mock_provenance_dependencies;
use std::str::FromStr;

const OWNER: &str = "tp1fzvmcykduaj48yfp87k9gu2xqm6u6urslrwy0c";
/// Valid Provenance bech32 so addr_validate passes in instantiate.
const ORACLE: &str = "tp1kzcmgmx0qmc37tcpxj32ftakfs2upm49xngh7m";
const REPO_TOKEN_CW20: &str = "tp1a07pq74jt05vfmjgk9ksdfkwakzk3cx78xx6sz";

/// Default for validation tests: bind an already-deployed repo token (no `SubMsg`).
fn default_instantiate_msg() -> InstantiateMsg {
    InstantiateMsg {
        contract_name: "pool-v2-demo".to_string(),
        description: "Test pool v2".to_string(),
        repo_token: RepoTokenConfig::Existing {
            repo_token_cw20_contract_address: REPO_TOKEN_CW20.to_string(),
        },
        lending_denom: Denom::new("uylds.fcc", 6u32), // u prefix => 1 ylds.fcc = 10^6 uylds.fcc
        rate_params: RateParamsV1 {
            target_rate: Decimal256::from_str("0.09").unwrap(),
            min_rate: Decimal256::from_str("0.0325").unwrap(),
            max_rate: Decimal256::from_str("0.20").unwrap(),
            kink_utilization: Decimal256::from_str("0.90").unwrap(),
            reserve_factor: Decimal256::from_str("0.005").unwrap(),
            seconds_per_year: 31_536_000,
        },
        lender_required_attrs: vec!["lender.kyc".to_string()],
        borrower_required_attrs: vec!["borrower.kyc".to_string()],
        price_oracle_address: ORACLE.to_string(),
        max_borrower_collateral_types: 5,
        margin_rate: Decimal256::from_str("0.80").unwrap(),
        liquidation_rate: Decimal256::from_str("0.90").unwrap(),
        liquidation_bonus_rate: Decimal256::from_ratio(102u128, 100u128), // 2%
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

fn new_repo_instantiate_msg() -> InstantiateMsg {
    let mut msg = default_instantiate_msg();
    msg.repo_token = RepoTokenConfig::New {
        repo_token_code_id: 1,
        repo_token_name: "Pool Repo".to_string(),
        repo_token_symbol: "pREPO".to_string(),
        repo_token_decimals: 6,
    };
    msg
}

#[test]
fn instantiate_existing_repo_token_no_submessage_stores_address() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let msg = default_instantiate_msg();

    let res = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .expect("instantiate should succeed");

    assert!(res.messages.is_empty());
    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(
        state.repo_token_cw20_address.as_ref().map(|a| a.as_str()),
        Some(REPO_TOKEN_CW20)
    );
}

#[test]
fn instantiate_success_stores_state_emits_repo_token_submsg_and_reply_binds_address() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let env = mock_env();
    let info = message_info(&Addr::unchecked(OWNER), &[]);
    let msg = new_repo_instantiate_msg();
    let expected_code_id = match &msg.repo_token {
        RepoTokenConfig::New {
            repo_token_code_id, ..
        } => *repo_token_code_id,
        _ => panic!("expected New variant"),
    };

    let res = instantiate_contract(deps.as_mut(), env.clone(), info.clone(), msg.clone())
        .expect("instantiate should succeed");

    assert_eq!(res.attributes.len(), 1);
    assert_eq!(res.attributes[0].key, ATTRIBUTE_ACTION_NAME);
    assert_eq!(res.attributes[0].value, "instantiate");

    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Wasm(WasmMsg::Instantiate { code_id, .. }) => {
            assert_eq!(*code_id, expected_code_id);
        }
        _ => panic!("expected WasmMsg::Instantiate for repo token"),
    }

    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    let ownership = get_ownership(deps.as_ref().storage).unwrap();
    assert_eq!(ownership.owner, Some(Addr::unchecked(OWNER)));
    assert_eq!(state.contract_name, msg.contract_name);
    assert!(state.repo_token_cw20_address.is_none());
    assert_eq!(state.lending_denom.name, msg.lending_denom.name);
    assert_eq!(state.min_lend, msg.min_lend);
    assert_eq!(state.min_borrow, msg.min_borrow);

    let reserve = get_reserve_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(reserve.total_scaled_liquidity, 0);
    assert_eq!(reserve.total_scaled_borrow, 0);
    assert!(reserve.liquidity_index > Decimal256::zero());
    assert!(reserve.borrow_index > Decimal256::zero());

    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(state.supported_collateral_assets.len(), 1);
    assert_eq!(state.supported_collateral_assets[0].asset_id, "asset.one");

    let ver = get_contract_version(deps.as_ref().storage).unwrap();
    assert_eq!(ver.contract, CONTRACT_NAME);
    assert_eq!(ver.version, CONTRACT_VERSION);

    reply(
        deps.as_mut(),
        env.clone(),
        mock_repo_token_instantiate_reply(REPO_TOKEN_CW20),
    )
    .expect("reply should succeed");

    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(
        state.repo_token_cw20_address.as_ref().map(|a| a.as_str()),
        Some(REPO_TOKEN_CW20)
    );
}

#[test]
fn instantiate_fails_zero_repo_token_code_id() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = new_repo_instantiate_msg();
    msg.repo_token = RepoTokenConfig::New {
        repo_token_code_id: 0,
        repo_token_name: "Pool Repo".to_string(),
        repo_token_symbol: "pREPO".to_string(),
        repo_token_decimals: 6,
    };

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("repo_token_code_id must be greater than zero"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_empty_repo_token_cw20_address() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.repo_token = RepoTokenConfig::Existing {
        repo_token_cw20_contract_address: "".to_string(),
    };

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("repo_token_cw20_contract_address cannot be empty"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_invalid_repo_token_cw20_address() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.repo_token = RepoTokenConfig::Existing {
        repo_token_cw20_contract_address: "not-a-valid-bech32-address".to_string(),
    };

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::Std(_) => {}
        _ => panic!(
            "expected Std (invalid address) from addr_validate, got {:?}",
            err
        ),
    }
}

#[test]
fn instantiate_with_existing_cw20_address_succeeds() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let msg = default_instantiate_msg();

    let res = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .expect("instantiate should succeed");
    assert!(res.messages.is_empty());
    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert_eq!(
        state.repo_token_cw20_address.unwrap().as_str(),
        REPO_TOKEN_CW20
    );
}

#[test]
fn instantiate_fails_repo_token_name_too_short() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = new_repo_instantiate_msg();
    msg.repo_token = RepoTokenConfig::New {
        repo_token_code_id: 1,
        repo_token_name: "ab".to_string(),
        repo_token_symbol: "pREPO".to_string(),
        repo_token_decimals: 6,
    };

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("repo_token:")
                    && message.contains("name must be 3–50 UTF-8 bytes"),
                "unexpected message: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_on_empty_contract_name() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.contract_name = "".to_string();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("contract_name cannot be empty"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_on_blank_contract_name() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.contract_name = " \t \n \r ".to_string();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("contract_name cannot be empty"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_empty_price_oracle_address() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.price_oracle_address = "".to_string();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("price_oracle_address cannot be empty"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_invalid_lending_denom_empty_name() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.lending_denom = Denom::new("", 6u32);

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Denom name cannot be empty"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_invalid_lending_denom_precision_too_high() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.lending_denom = Denom::new("uylds.fcc", 60u32); // precision too high

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("precision"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_invalid_collateral_asset_empty_id() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.supported_collateral_assets = vec![CollateralAssetV1 {
        asset_id: "".to_string(),
        haircut: None,
    }];

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Asset ID cannot be empty"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_when_collateral_asset_id_is_lending_denom() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    let lending = msg.lending_denom.name.clone();
    msg.supported_collateral_assets = vec![CollateralAssetV1 {
        asset_id: lending.clone(),
        haircut: Some(Decimal256::percent(80)),
    }];

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
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
fn instantiate_fails_duplicate_supported_collateral_asset_id() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.supported_collateral_assets = vec![
        CollateralAssetV1 {
            asset_id: "asset.one".to_string(),
            haircut: Some(Decimal256::percent(80)),
        },
        CollateralAssetV1 {
            asset_id: "asset.one".to_string(),
            haircut: Some(Decimal256::percent(90)),
        },
    ];

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("Duplicate supported_collateral_assets"));
            assert!(message.contains("asset.one"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_min_lend_zero() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.min_lend = Uint128::zero();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
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
fn instantiate_fails_min_borrow_zero() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.min_borrow = Uint128::zero();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("min_borrow must be at least 1"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_max_borrower_collateral_types_zero() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.max_borrower_collateral_types = 0;

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
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
fn instantiate_fails_rate_params_min_above_target() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.rate_params.min_rate = Decimal256::from_str("0.10").unwrap();
    msg.rate_params.target_rate = Decimal256::from_str("0.09").unwrap();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("rate_params"));
            assert!(message.contains("min_rate"));
            assert!(message.contains("target_rate"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_rate_params_target_above_max() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.rate_params.target_rate = Decimal256::from_str("0.25").unwrap();
    msg.rate_params.max_rate = Decimal256::from_str("0.20").unwrap();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("rate_params"));
            assert!(message.contains("target_rate"));
            assert!(message.contains("max_rate"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_rate_params_kink_zero() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.rate_params.kink_utilization = Decimal256::zero();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
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
fn instantiate_fails_rate_params_reserve_factor_one() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.rate_params.reserve_factor = Decimal256::one();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
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
fn instantiate_fails_rate_params_seconds_per_year_zero() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.rate_params.seconds_per_year = 0;

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("seconds_per_year"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_when_margin_rate_zero() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.margin_rate = Decimal256::zero();
    msg.liquidation_rate = Decimal256::from_str("0.90").unwrap();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("margin_rate must be greater than zero"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_when_liquidation_rate_zero() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.margin_rate = Decimal256::from_str("0.80").unwrap();
    msg.liquidation_rate = Decimal256::zero();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("liquidation_rate must be greater than zero"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_when_liquidation_rate_is_greater_than_one() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.margin_rate = Decimal256::from_str("0.80").unwrap();
    msg.liquidation_rate = Decimal256::from_str("1.000001").unwrap();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(message.contains("liquidation_rate must be less than or equal to 1"));
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_when_margin_rate_not_less_than_liquidation_rate() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.margin_rate = Decimal256::from_str("0.90").unwrap();
    msg.liquidation_rate = Decimal256::from_str("0.90").unwrap();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
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
fn instantiate_fails_when_bonus_times_margin_rate_ge_one() {
    // 1 - liquidation_bonus_rate * margin_rate must be positive for min-repay formula.
    // bonus=1.02, margin_rate=0.99 => 1.0098 >= 1 => denominator would underflow.
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.liquidation_bonus_rate = Decimal256::from_ratio(102u128, 100u128); // 1.02
    msg.margin_rate = Decimal256::from_str("0.99").unwrap();
    msg.liquidation_rate = Decimal256::one(); // still margin < liquidation

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("liquidation_bonus_rate * margin_rate must be < 1"),
                "expected message about bonus*margin < 1, got: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_succeeds_with_empty_required_attrs() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.lender_required_attrs = vec![];
    msg.borrower_required_attrs = vec![];

    let res = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .expect("instantiate should succeed");

    assert!(res.messages.is_empty());
    let state = get_contract_state_v1(deps.as_ref().storage).unwrap();
    assert!(state.lender_required_attrs.is_empty());
    assert!(state.borrower_required_attrs.is_empty());
}

#[test]
fn instantiate_fails_when_lender_required_attrs_exceed_limit() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.lender_required_attrs = (0..MAX_LENDER_BORROWER_REQUIRED_ATTRS + 1)
        .map(|i| format!("lender.{i}"))
        .collect();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("lender required attributes")
                    && message.contains(&MAX_LENDER_BORROWER_REQUIRED_ATTRS.to_string()),
                "unexpected message: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}

#[test]
fn instantiate_fails_when_borrower_required_attrs_exceed_limit() {
    let mut deps = mock_provenance_dependencies();
    deps.api = deps.api.with_prefix("tp");
    let mut msg = default_instantiate_msg();
    msg.borrower_required_attrs = (0..MAX_LENDER_BORROWER_REQUIRED_ATTRS + 1)
        .map(|i| format!("borrower.{i}"))
        .collect();

    let err = instantiate_contract(
        deps.as_mut(),
        mock_env(),
        message_info(&Addr::unchecked(OWNER), &[]),
        msg,
    )
    .unwrap_err();

    match &err {
        ContractError::IllegalArgumentError { message } => {
            assert!(
                message.contains("borrower required attributes")
                    && message.contains(&MAX_LENDER_BORROWER_REQUIRED_ATTRS.to_string()),
                "unexpected message: {}",
                message
            );
        }
        _ => panic!("expected IllegalArgumentError, got {:?}", err),
    }
}
